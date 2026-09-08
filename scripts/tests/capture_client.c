/* Live compositor probe. Run on a quiet disposable desktop, under timeout.
 * Modes: damage (no duplicate static frame), invalid (wrong SHM format),
 * reuse (one-shot frame enforcement), lock (lock after PENDING).
 * Generate capture-protocol.[ch] from wlr-screencopy-unstable-v1.xml.
 * See scripts/tests/README.md for the build and VM commands.
 */
#define _GNU_SOURCE
#include <wayland-client.h>
#include <sys/mman.h>
#include <unistd.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "capture-protocol.h"
static struct wl_display *display;
static struct wl_shm *shm;
static struct wl_output *output;
static struct zwlr_screencopy_manager_v1 *manager;
static struct wl_buffer *buffer;
static int ready,failed,advertised,format,width,height,stride;
static void globals(void *d,struct wl_registry *r,uint32_t name,const char *i,uint32_t v) {
 if(!strcmp(i,"wl_shm")) shm=wl_registry_bind(r,name,&wl_shm_interface,1);
 if(!strcmp(i,"wl_output")&&!output) output=wl_registry_bind(r,name,&wl_output_interface,1);
 if(!strcmp(i,"zwlr_screencopy_manager_v1")) manager=wl_registry_bind(r,name,&zwlr_screencopy_manager_v1_interface,3);
}
static void removed(void*d,struct wl_registry*r,uint32_t n){}
static const struct wl_registry_listener registry_listener={globals,removed};
static void frame_buffer(void*d,struct zwlr_screencopy_frame_v1*f,uint32_t fmt,uint32_t w,uint32_t h,uint32_t s){format=fmt;width=w;height=h;stride=s;}
static void frame_flags(void*d,struct zwlr_screencopy_frame_v1*f,uint32_t flags){}
static void frame_ready(void*d,struct zwlr_screencopy_frame_v1*f,uint32_t h,uint32_t l,uint32_t ns){ready++;}
static void frame_failed(void*d,struct zwlr_screencopy_frame_v1*f){failed++;}
static void frame_damage(void*d,struct zwlr_screencopy_frame_v1*f,uint32_t x,uint32_t y,uint32_t w,uint32_t h){}
static void frame_dmabuf(void*d,struct zwlr_screencopy_frame_v1*f,uint32_t format,uint32_t w,uint32_t h){}
static void frame_done(void*d,struct zwlr_screencopy_frame_v1*f){advertised=1;}
static const struct zwlr_screencopy_frame_v1_listener listener={frame_buffer,frame_flags,frame_ready,frame_failed,frame_damage,frame_dmabuf,frame_done};
static long now_ms(void){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec*1000+t.tv_nsec/1000000;}
static int dispatch_for(int milliseconds,int expected){
 long deadline=now_ms()+milliseconds;
 while(now_ms()<deadline&&!failed&&ready<expected){
  if(wl_display_dispatch_pending(display)<0)return -1;
  wl_display_flush(display);
  struct pollfd p={wl_display_get_fd(display),POLLIN,0};
  long remaining=deadline-now_ms();
  if(remaining<=0)break;
  int result=poll(&p,1,(int)remaining);
  if(result>0&&wl_display_dispatch(display)<0)return -1;
 }
 return ready;
}
static struct zwlr_screencopy_frame_v1 *capture(void){
 advertised=0;
 struct zwlr_screencopy_frame_v1 *f=zwlr_screencopy_manager_v1_capture_output(manager,0,output);
 zwlr_screencopy_frame_v1_add_listener(f,&listener,NULL);
 while(!advertised&&!failed)if(wl_display_dispatch(display)<0)exit(2);
 return f;
}
int main(int argc,char **argv){
 const char *mode=argc>1?argv[1]:"damage";
 display=wl_display_connect(NULL);if(!display)return 2;
 struct wl_registry*r=wl_display_get_registry(display);wl_registry_add_listener(r,&registry_listener,NULL);wl_display_roundtrip(display);
 if(!shm||!manager||!output)return 3;
 struct zwlr_screencopy_frame_v1 *f=capture();if(failed)return 4;
 int fd=memfd_create("capture-test",MFD_CLOEXEC);size_t bytes=(size_t)stride*height;
 ftruncate(fd,bytes);
 struct wl_shm_pool*p=wl_shm_create_pool(shm,fd,bytes);
 int invalid=!strcmp(mode,"invalid");
 buffer=wl_shm_pool_create_buffer(p,0,width,height,stride,invalid?WL_SHM_FORMAT_ARGB8888:format);
 wl_shm_pool_destroy(p);close(fd);
 zwlr_screencopy_frame_v1_copy_with_damage(f,buffer);
 if(invalid){int result=dispatch_for(2000,1);printf("invalid buffer disconnected=%d\n",result<0);return result<0?0:5;}
 if(dispatch_for(3000,1)!=1)return 6;
 if(!strcmp(mode,"reuse")){
  zwlr_screencopy_frame_v1_copy(f,buffer);int result=dispatch_for(2000,2);
  printf("reused frame disconnected=%d\n",result<0);return result<0?0:7;
 }
 zwlr_screencopy_frame_v1_destroy(f);f=capture();zwlr_screencopy_frame_v1_copy_with_damage(f,buffer);
 if(!strcmp(mode,"lock")){
  puts("PENDING");fflush(stdout);dispatch_for(10000,2);
  printf("locked pending failed=%d extra_frames=%d\n",failed,ready-1);return failed&&ready==1?0:9;
 }
 int result=dispatch_for(500,2);
 printf("static screen frames=%d (expected 1)\n",result);
 zwlr_screencopy_frame_v1_destroy(f);wl_buffer_destroy(buffer);wl_display_flush(display);wl_display_disconnect(display);
 return result==1?0:8;
}
