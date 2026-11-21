/*
 * Support for macOS tests of IOSurface and the IOKit calls Chrome makes at start-up. A test that includes this header says
 * `poly-mac: iosurface` in its header comment (`mac build` then links IOSurface, IOKit and CoreFoundation and lets the test import their symbols
 * and libSystem's), includes this file, and defines `int test_main(void)`. Output goes through raw write calls (poly_mlibc.h): ok / not ok /
 * value / summary.
 *
 * Tests never print what is the owner's machine's own: not the serial number, not the platform UUID, not the registry's names of this
 * Mac, not numbers that depend on the hardware (how many services of a class there are, a GPU's core count). A test about IOKit records
 * shapes: whether a call finds something, the type of a property, the length and the character classes of a string, the error codes.
 *
 * Tests that share a surface between two processes run themselves again as a child (is_child) and exchange the surface's Mach port through a
 * bootstrap service of a name of their own (ios_service_name), as Chrome's processes do through their rendezvous.
 *
 * Written for Poly from public sources (the SDK headers IOSurface/IOSurfaceRef.h, IOKit/IOKitLib.h) and from observation only. No Apple or
 * third-party source was read.
 */
#ifndef POLY_MIOSURFACE_H
#define POLY_MIOSURFACE_H

#include "poly_mlibc.h"

#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <IOSurface/IOSurface.h>
#include <mach/mach.h>
#include <servers/bootstrap.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

/* A dictionary with the properties of a surface of width x height elements of bytes_per_element bytes (and a pixel format when it is not 0).
 * More properties can be added with ios_set_int. */
static inline void ios_set_int(CFMutableDictionaryRef d, CFStringRef key, long long v) {
    CFNumberRef n = CFNumberCreate(NULL, kCFNumberLongLongType, &v);
    CFDictionarySetValue(d, key, n);
    CFRelease(n);
}
static inline CFMutableDictionaryRef ios_props(long long w, long long h, long long bpe, unsigned format) {
    CFMutableDictionaryRef d = CFDictionaryCreateMutable(NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    if (w >= -1000000) ios_set_int(d, kIOSurfaceWidth, w);
    if (h >= -1000000) ios_set_int(d, kIOSurfaceHeight, h);
    if (bpe >= -1000000) ios_set_int(d, kIOSurfaceBytesPerElement, bpe);
    if (format) ios_set_int(d, kIOSurfacePixelFormat, format);
    return d;
}
/* The same, as a surface (NULL when IOSurfaceCreate says NULL); the dictionary is released. */
static inline IOSurfaceRef ios_make(long long w, long long h, long long bpe) {
    CFMutableDictionaryRef d = ios_props(w, h, bpe, 'BGRA');
    IOSurfaceRef s = IOSurfaceCreate(d);
    CFRelease(d);
    return s;
}
static inline IOSurfaceRef ios_make_props(CFMutableDictionaryRef d) {
    IOSurfaceRef s = IOSurfaceCreate(d);
    CFRelease(d);
    return s;
}
static inline long long ios_num(CFTypeRef v) {
    long long n = -1;
    if (v && CFGetTypeID(v) == CFNumberGetTypeID()) CFNumberGetValue(v, kCFNumberLongLongType, &n);
    return n;
}
static inline int ios_str_eq(CFTypeRef v, const char *s) {
    if (!v || CFGetTypeID(v) != CFStringGetTypeID()) return 0;
    CFStringRef t = CFStringCreateWithCString(NULL, s, kCFStringEncodingUTF8);
    int eq = CFEqual(v, t);
    CFRelease(t);
    return eq;
}
static inline CFStringRef ios_cfstr(const char *s) { return CFStringCreateWithCString(NULL, s, kCFStringEncodingUTF8); }

/* Children, started by running this program again with a mode and an argument. */
static inline pid_t ios_start_self(const char *mode, const char *arg) {
    char *argv[4] = {poly_argv[0], (char *)mode, (char *)arg, NULL};
    pid_t pid = -1;
    return posix_spawn(&pid, poly_argv[0], NULL, NULL, argv, environ) ? -1 : pid;
}
static inline int ios_wait(pid_t pid) {
    int st = 0;
    if (waitpid(pid, &st, 0) != pid) return -1;
    return WIFEXITED(st) ? WEXITSTATUS(st) : 1000 + WTERMSIG(st);
}
static inline const char *ios_child_arg(void) { return poly_argc > 2 ? poly_argv[2] : ""; }
static inline void ios_service_name(char *buf, unsigned long n, const char *tag) { snprintf(buf, n, "com.poly.spec.ios.%s.%d", tag, (int)getpid()); }

/* A surface's Mach port sent to the receive right `to` (a name this process has a send right to), as a port descriptor. */
typedef struct {
    mach_msg_header_t h;
    mach_msg_body_t body;
    mach_msg_port_descriptor_t port;
} ios_port_msg_t;
typedef struct {
    ios_port_msg_t m;
    mach_msg_trailer_t t;
} ios_port_rcv_t;
static inline kern_return_t ios_send_port(mach_port_t to, mach_port_t port, int id) {
    ios_port_msg_t m;
    memset(&m, 0, sizeof m);
    m.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
    m.h.msgh_remote_port = to;
    m.h.msgh_size = sizeof m;
    m.h.msgh_id = id;
    m.body.msgh_descriptor_count = 1;
    m.port.name = port;
    m.port.disposition = MACH_MSG_TYPE_COPY_SEND;
    m.port.type = MACH_MSG_PORT_DESCRIPTOR;
    return mach_msg(&m.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof m, 0, MACH_PORT_NULL, 5000, MACH_PORT_NULL);
}
/* Receive a port sent with ios_send_port on the receive right `rp`; the port in *out. */
static inline kern_return_t ios_receive_port(mach_port_t rp, mach_port_t *out) {
    ios_port_rcv_t r;
    memset(&r, 0, sizeof r);
    kern_return_t kr = mach_msg(&r.m.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, rp, 8000, MACH_PORT_NULL);
    if (kr == KERN_SUCCESS) *out = r.m.port.name;
    return kr;
}
/* Look a bootstrap name up until it exists (the other process checks it in); the send right in *out. */
static inline kern_return_t ios_wait_service(const char *name, mach_port_t *out) {
    kern_return_t kr = 1;
    for (int i = 0; i < 160 && kr; i++) {
        kr = bootstrap_look_up(bootstrap_port, (char *)name, out);
        if (kr) usleep(50000);
    }
    return kr;
}

/* The type of a CF value, as a short word. */
static inline const char *ios_type_name(CFTypeRef v) {
    if (!v) return "NULL";
    CFTypeID t = CFGetTypeID(v);
    if (t == CFStringGetTypeID()) return "string";
    if (t == CFNumberGetTypeID()) return "number";
    if (t == CFDataGetTypeID()) return "data";
    if (t == CFDictionaryGetTypeID()) return "dict";
    if (t == CFArrayGetTypeID()) return "array";
    if (t == CFBooleanGetTypeID()) return "bool";
    return "other";
}
static inline long ios_str_len(CFTypeRef v) { return v && CFGetTypeID(v) == CFStringGetTypeID() ? (long)CFStringGetLength(v) : -1; }

/* The number of objects an iterator gives (the iterator is released); -1 for an iterator that is not set. */
static inline int ios_drain(io_iterator_t it) {
    if (!it) return -1;
    int n = 0;
    io_object_t o;
    while ((o = IOIteratorNext(it))) {
        n++;
        IOObjectRelease(o);
    }
    IOObjectRelease(it);
    return n;
}
/* The number of registry services of a class. */
static inline int ios_count_services(const char *cls) {
    io_iterator_t it = 0;
    if (IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching(cls), &it) != KERN_SUCCESS) return -2;
    return ios_drain(it);
}

#endif
