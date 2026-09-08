/* Bounded by the VM controller. A real Wayland client for keyboard/switching QA. */
#define _GNU_SOURCE
#include <wayland-client.h>
#include <sys/mman.h>
#include <unistd.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "xdg-shell-protocol.h"
#include "keyboard-shortcuts-inhibit-protocol.h"

static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct wl_seat *seat;
static struct xdg_wm_base *wm;
static struct zwp_keyboard_shortcuts_inhibit_manager_v1 *manager;
static struct wl_surface *surface;
static int width=640, height=420;
static uint32_t color;

static void keymap(void*d,struct wl_keyboard*k,uint32_t format,int fd,uint32_t size){close(fd);}
static void enter(void*d,struct wl_keyboard*k,uint32_t serial,struct wl_surface*s,struct wl_array*keys){puts("FOCUS");}
static void leave(void*d,struct wl_keyboard*k,uint32_t serial,struct wl_surface*s){puts("BLUR");}
static void key(void*d,struct wl_keyboard*k,uint32_t serial,uint32_t time,uint32_t key,uint32_t state){printf("KEY %u %u\n",key,state);}
static void modifiers(void*d,struct wl_keyboard*k,uint32_t serial,uint32_t depressed,uint32_t latched,uint32_t locked,uint32_t group){}
static void repeat(void*d,struct wl_keyboard*k,int32_t rate,int32_t delay){}
static const struct wl_keyboard_listener keyboard_listener={keymap,enter,leave,key,modifiers,repeat};
static void ping(void*d,struct xdg_wm_base*w,uint32_t serial){xdg_wm_base_pong(w,serial);}
static const struct xdg_wm_base_listener wm_listener={ping};
static void active(void*d,struct zwp_keyboard_shortcuts_inhibitor_v1*i){puts("INHIBITED");}
static void inactive(void*d,struct zwp_keyboard_shortcuts_inhibitor_v1*i){puts("UNINHIBITED");}
static const struct zwp_keyboard_shortcuts_inhibitor_v1_listener inhibit_listener={active,inactive};
static void global(void*d,struct wl_registry*r,uint32_t name,const char*interface,uint32_t version){
 if(!strcmp(interface,"wl_compositor"))compositor=wl_registry_bind(r,name,&wl_compositor_interface,4);
 if(!strcmp(interface,"wl_shm"))shm=wl_registry_bind(r,name,&wl_shm_interface,1);
 if(!strcmp(interface,"wl_seat"))seat=wl_registry_bind(r,name,&wl_seat_interface,4);
 if(!strcmp(interface,"xdg_wm_base")){wm=wl_registry_bind(r,name,&xdg_wm_base_interface,1);xdg_wm_base_add_listener(wm,&wm_listener,NULL);}
 if(!strcmp(interface,"zwp_keyboard_shortcuts_inhibit_manager_v1"))manager=wl_registry_bind(r,name,&zwp_keyboard_shortcuts_inhibit_manager_v1_interface,1);
}
static void removed(void*d,struct wl_registry*r,uint32_t name){}
static const struct wl_registry_listener registry_listener={global,removed};
static void released(void*d,struct wl_buffer*b){wl_buffer_destroy(b);}
static const struct wl_buffer_listener buffer_listener={released};
static void configure(void*d,struct xdg_surface*x,uint32_t serial){
 xdg_surface_ack_configure(x,serial);
 size_t size=(size_t)width*height*4;
 int fd=memfd_create("keyboard-probe",MFD_CLOEXEC);
 if(fd<0||ftruncate(fd,size))exit(4);
 uint32_t *pixels=mmap(NULL,size,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);
 if(pixels==MAP_FAILED)exit(4);
 for(size_t i=0;i<size/4;i++)pixels[i]=color;
 struct wl_shm_pool *pool=wl_shm_create_pool(shm,fd,size);
 struct wl_buffer *buffer=wl_shm_pool_create_buffer(pool,0,width,height,width*4,WL_SHM_FORMAT_XRGB8888);
 wl_buffer_add_listener(buffer,&buffer_listener,NULL);
 wl_shm_pool_destroy(pool);munmap(pixels,size);close(fd);
 wl_surface_attach(surface,buffer,0,0);wl_surface_damage(surface,0,0,width,height);wl_surface_commit(surface);
 printf("MAPPED %d %d\n",width,height);
}
static const struct xdg_surface_listener surface_listener={configure};
static void top_configure(void*d,struct xdg_toplevel*t,int32_t w,int32_t h,struct wl_array*states){
 if(w>0)width=w;if(h>0)height=h;
 uint32_t *state;int fullscreen=0;
 wl_array_for_each(state,states)if(*state==XDG_TOPLEVEL_STATE_FULLSCREEN)fullscreen=1;
 printf("FULLSCREEN %d\n",fullscreen);
}
static void close_window(void*d,struct xdg_toplevel*t){puts("CLOSED");exit(0);}
static const struct xdg_toplevel_listener top_listener={top_configure,close_window};

int main(int argc,char**argv){
 if(argc!=4)return 2;
 setvbuf(stdout,NULL,_IOLBF,0);color=0xff000000u|strtoul(argv[3],NULL,16);
 display=wl_display_connect(NULL);if(!display)return 2;
 struct wl_registry *registry=wl_display_get_registry(display);
 wl_registry_add_listener(registry,&registry_listener,NULL);wl_display_roundtrip(display);
 if(!compositor||!shm||!seat||!wm||!manager)return 3;
 struct wl_keyboard *keyboard=wl_seat_get_keyboard(seat);wl_keyboard_add_listener(keyboard,&keyboard_listener,NULL);
 surface=wl_compositor_create_surface(compositor);
 struct xdg_surface *xdg=xdg_wm_base_get_xdg_surface(wm,surface);xdg_surface_add_listener(xdg,&surface_listener,NULL);
 struct xdg_toplevel *top=xdg_surface_get_toplevel(xdg);xdg_toplevel_add_listener(top,&top_listener,NULL);
 xdg_toplevel_set_title(top,argv[1]);xdg_toplevel_set_app_id(top,"mindos.keyboard-probe");
 if(atoi(argv[2])){
  struct zwp_keyboard_shortcuts_inhibitor_v1 *inhibitor=zwp_keyboard_shortcuts_inhibit_manager_v1_inhibit_shortcuts(manager,surface,seat);
  zwp_keyboard_shortcuts_inhibitor_v1_add_listener(inhibitor,&inhibit_listener,NULL);
 }
 wl_surface_commit(surface);
 while(wl_display_dispatch(display)>=0){}return 5;
}
