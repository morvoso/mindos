/* XWayland peer for the window-switching/WM_DELETE_WINDOW integration tests. */
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    if (argc != 3) return 2;
    setvbuf(stdout, NULL, _IOLBF, 0);
    Display *display = XOpenDisplay(NULL);
    if (!display) return 3;
    int screen = DefaultScreen(display);
    Window window = XCreateSimpleWindow(display, RootWindow(display, screen),
        100, 100, 640, 420, 0, 0, strtoul(argv[2], NULL, 16));
    XStoreName(display, window, argv[1]);
    XClassHint hint = {"mindos.keyboard-probe-x11", "MindOSKeyboardProbe"};
    XSetClassHint(display, window, &hint);
    Atom close = XInternAtom(display, "WM_DELETE_WINDOW", False);
    XSetWMProtocols(display, window, &close, 1);
    XSelectInput(display, window, StructureNotifyMask | KeyPressMask | KeyReleaseMask | FocusChangeMask);
    XMapWindow(display, window);
    XFlush(display);
    for (;;) {
        XEvent event;
        XNextEvent(display, &event);
        if (event.type == MapNotify) puts("MAPPED");
        if (event.type == FocusIn) puts("FOCUS");
        if (event.type == FocusOut) puts("BLUR");
        if (event.type == KeyPress || event.type == KeyRelease)
            printf("KEY %u %d\n", event.xkey.keycode, event.type == KeyPress);
        if (event.type == ClientMessage && (Atom)event.xclient.data.l[0] == close) {
            puts("CLOSED");
            XDestroyWindow(display, window);
            XCloseDisplay(display);
            return 0;
        }
    }
}
