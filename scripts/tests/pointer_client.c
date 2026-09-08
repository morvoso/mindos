/* Interactive protocol probe for a disposable desktop. See README.md.
 * lock / confine: persistent constraint with a bounded region and a hole.
 * The first key press releases the constraint; Escape then closes the probe.
 */
#define _GNU_SOURCE
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include <sys/mman.h>
#include <unistd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "pointer-constraints-protocol.h"
#include "relative-pointer-protocol.h"
#include "xdg-shell-protocol.h"
static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct wl_seat *seat;
static struct wl_pointer *pointer;
static struct wl_keyboard *keyboard;
static struct xkb_context *xkb_context;
static struct xkb_state *xkb_state;
static struct wl_surface *surface;
static struct xdg_wm_base *wm;
static struct xdg_surface *xdg;
static struct zwp_pointer_constraints_v1 *constraints;
static struct zwp_relative_pointer_manager_v1 *relative_manager;
static struct zwp_locked_pointer_v1 *locked;
static struct zwp_confined_pointer_v1 *confined;
static int width=1920,height=1080,active,configured,created;
static const char *mode;
static void on_locked(void*d,struct zwp_locked_pointer_v1*p){active=1;puts("LOCKED");zwp_locked_pointer_v1_set_cursor_position_hint(p,wl_fixed_from_int(20),wl_fixed_from_int(20));wl_surface_commit(surface);}
static void on_unlocked(void*d,struct zwp_locked_pointer_v1*p){active=0;puts("UNLOCKED");}
static const struct zwp_locked_pointer_v1_listener lock_listener={on_locked,on_unlocked};
static void on_confined(void*d,struct zwp_confined_pointer_v1*p){active=1;puts("CONFINED");}
static void on_unconfined(void*d,struct zwp_confined_pointer_v1*p){active=0;puts("UNCONFINED");}
static const struct zwp_confined_pointer_v1_listener confine_listener={on_confined,on_unconfined};
static void enter(void*d,struct wl_pointer*p,uint32_t serial,struct wl_surface*s,wl_fixed_t x,wl_fixed_t y){
 printf("ENTER %.4f %.4f\n",wl_fixed_to_double(x),wl_fixed_to_double(y));
 if(created)return;created=1;
 struct wl_region*r=wl_compositor_create_region(compositor);
 wl_region_add(r,100,100,500,500);wl_region_subtract(r,300,100,50,500);
 if(!strcmp(mode,"lock")) {locked=zwp_pointer_constraints_v1_lock_pointer(constraints,s,p,r,ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_PERSISTENT);zwp_locked_pointer_v1_add_listener(locked,&lock_listener,NULL);}
 if(!strcmp(mode,"confine")) {confined=zwp_pointer_constraints_v1_confine_pointer(constraints,s,p,r,ZWP_POINTER_CONSTRAINTS_V1_LIFETIME_PERSISTENT);zwp_confined_pointer_v1_add_listener(confined,&confine_listener,NULL);}
 wl_region_destroy(r);wl_surface_commit(s);puts("CREATED");
}
static void leave(void*d,struct wl_pointer*p,uint32_t serial,struct wl_surface*s){puts("LEAVE");}
static void motion(void*d,struct wl_pointer*p,uint32_t time,wl_fixed_t x,wl_fixed_t y){printf("MOTION %.4f %.4f active=%d\n",wl_fixed_to_double(x),wl_fixed_to_double(y),active);}
static void button(void*d,struct wl_pointer*p,uint32_t s,uint32_t t,uint32_t b,uint32_t state){printf("BUTTON %u %u\n",b,state);}
static void axis(void*d,struct wl_pointer*p,uint32_t t,uint32_t a,wl_fixed_t v){}
static const struct wl_pointer_listener pointer_listener={.enter=enter,.leave=leave,.motion=motion,.button=button,.axis=axis};
static void relative(void*d,struct zwp_relative_pointer_v1*p,uint32_t hi,uint32_t lo,wl_fixed_t x,wl_fixed_t y,wl_fixed_t ux,wl_fixed_t uy){printf("RELATIVE %.4f %.4f raw=%.4f,%.4f\n",wl_fixed_to_double(x),wl_fixed_to_double(y),wl_fixed_to_double(ux),wl_fixed_to_double(uy));}
static const struct zwp_relative_pointer_v1_listener relative_listener={relative};
static void keymap(void*d,struct wl_keyboard*k,uint32_t f,int fd,uint32_t size){
 if(f!=WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1){close(fd);return;}
 char *text=mmap(NULL,size,PROT_READ,MAP_PRIVATE,fd,0);close(fd);
 if(text==MAP_FAILED)exit(6);
 struct xkb_keymap *map=xkb_keymap_new_from_string(xkb_context,text,XKB_KEYMAP_FORMAT_TEXT_V1,XKB_KEYMAP_COMPILE_NO_FLAGS);
 munmap(text,size);if(!map)exit(7);
 if(xkb_state)xkb_state_unref(xkb_state);
 xkb_state=xkb_state_new(map);xkb_keymap_unref(map);puts("KEYMAP");
}
static void key_enter(void*d,struct wl_keyboard*k,uint32_t s,struct wl_surface*w,struct wl_array*a){}
static void key_leave(void*d,struct wl_keyboard*k,uint32_t s,struct wl_surface*w){}
static void key(void*d,struct wl_keyboard*k,uint32_t s,uint32_t t,uint32_t key,uint32_t state){
 if(!state)return;
 if(xkb_state){char text[64];xkb_state_key_get_utf8(xkb_state,key+8,text,sizeof(text));printf("KEY %u %s\n",key,text);}
 if(locked){zwp_locked_pointer_v1_destroy(locked);locked=NULL;active=0;puts("RELEASED");}
 else if(confined){zwp_confined_pointer_v1_destroy(confined);confined=NULL;active=0;puts("RELEASED");}
 else if(key==1)exit(0);
}
static void modifiers(void*d,struct wl_keyboard*k,uint32_t s,uint32_t a,uint32_t b,uint32_t c,uint32_t g){if(xkb_state)xkb_state_update_mask(xkb_state,a,b,c,0,0,g);}
static void repeat(void*d,struct wl_keyboard*k,int32_t r,int32_t delay){printf("REPEAT %d %d\n",r,delay);}
static const struct wl_keyboard_listener keyboard_listener={keymap,key_enter,key_leave,key,modifiers,repeat};
static void ping(void*d,struct xdg_wm_base*w,uint32_t s){xdg_wm_base_pong(w,s);}
static const struct xdg_wm_base_listener wm_listener={ping};
static void globals(void*d,struct wl_registry*r,uint32_t n,const char*i,uint32_t v){
 if(!strcmp(i,"wl_compositor"))compositor=wl_registry_bind(r,n,&wl_compositor_interface,4);
 if(!strcmp(i,"wl_shm"))shm=wl_registry_bind(r,n,&wl_shm_interface,1);
 if(!strcmp(i,"wl_seat"))seat=wl_registry_bind(r,n,&wl_seat_interface,4);
 if(!strcmp(i,"xdg_wm_base")){wm=wl_registry_bind(r,n,&xdg_wm_base_interface,1);xdg_wm_base_add_listener(wm,&wm_listener,NULL);}
 if(!strcmp(i,"zwp_pointer_constraints_v1"))constraints=wl_registry_bind(r,n,&zwp_pointer_constraints_v1_interface,1);
 if(!strcmp(i,"zwp_relative_pointer_manager_v1"))relative_manager=wl_registry_bind(r,n,&zwp_relative_pointer_manager_v1_interface,1);
}
static void removed(void*d,struct wl_registry*r,uint32_t n){}
static const struct wl_registry_listener registry_listener={globals,removed};
static void configure(void*d,struct xdg_surface*s,uint32_t serial){
 xdg_surface_ack_configure(s,serial);if(configured)return;configured=1;
 int fd=memfd_create("pointer-probe",MFD_CLOEXEC);size_t size=(size_t)width*height*4;
 if(fd<0||ftruncate(fd,size))exit(4);
 uint32_t*p=mmap(NULL,size,PROT_READ|PROT_WRITE,MAP_SHARED,fd,0);if(p==MAP_FAILED)exit(4);
 for(int y=0;y<height;y++)for(int x=0;x<width;x++)p[y*width+x]=(x>=100&&x<600&&y>=100&&y<600&&!(x>=300&&x<350))?0xff204c50:0xff101820;
 struct wl_shm_pool*pool=wl_shm_create_pool(shm,fd,size);
 struct wl_buffer*b=wl_shm_pool_create_buffer(pool,0,width,height,width*4,WL_SHM_FORMAT_XRGB8888);
 wl_shm_pool_destroy(pool);munmap(p,size);close(fd);
 wl_surface_attach(surface,b,0,0);wl_surface_damage(surface,0,0,width,height);wl_surface_commit(surface);puts("MAPPED");
}
static const struct xdg_surface_listener surface_listener={configure};
static void top_configure(void*d,struct xdg_toplevel*t,int32_t w,int32_t h,struct wl_array*s){if(w>0)width=w;if(h>0)height=h;uint32_t*state;int fullscreen=0;wl_array_for_each(state,s)if(*state==XDG_TOPLEVEL_STATE_FULLSCREEN)fullscreen=1;printf("FULLSCREEN %d\n",fullscreen);}
static void top_close(void*d,struct xdg_toplevel*t){exit(0);}
static const struct xdg_toplevel_listener top_listener={top_configure,top_close};
int main(int argc,char**argv){
 xkb_context=xkb_context_new(XKB_CONTEXT_NO_FLAGS);
 setvbuf(stdout,NULL,_IOLBF,0);mode=argc>1?argv[1]:"lock";
 display=wl_display_connect(NULL);if(!display)return 2;
 struct wl_registry*r=wl_display_get_registry(display);wl_registry_add_listener(r,&registry_listener,NULL);wl_display_roundtrip(display);
 if(!compositor||!shm||!seat||!wm||!constraints||!relative_manager)return 3;
 pointer=wl_seat_get_pointer(seat);wl_pointer_add_listener(pointer,&pointer_listener,NULL);
 keyboard=wl_seat_get_keyboard(seat);wl_keyboard_add_listener(keyboard,&keyboard_listener,NULL);
 struct zwp_relative_pointer_v1*rel=zwp_relative_pointer_manager_v1_get_relative_pointer(relative_manager,pointer);zwp_relative_pointer_v1_add_listener(rel,&relative_listener,NULL);
 surface=wl_compositor_create_surface(compositor);xdg=xdg_wm_base_get_xdg_surface(wm,surface);xdg_surface_add_listener(xdg,&surface_listener,NULL);
 struct xdg_toplevel*top=xdg_surface_get_toplevel(xdg);xdg_toplevel_add_listener(top,&top_listener,NULL);xdg_toplevel_set_title(top,"MindOS pointer probe");xdg_toplevel_set_app_id(top,"mindos.pointer-probe");xdg_toplevel_set_fullscreen(top,NULL);wl_surface_commit(surface);
 while(wl_display_dispatch(display)>=0){}return 5;
}
