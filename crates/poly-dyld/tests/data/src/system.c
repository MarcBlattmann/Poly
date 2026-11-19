/* A stand-in for /usr/lib/libSystem.B.dylib, written for Poly's tests: the
 * three names the test programs and their linker need, nothing of Apple's
 * library. `exit` ends the process with the raw call; the other two exist
 * only so the static linker can resolve them (Poly's loader replaces the
 * thread-local thunk and binds lazy pointers itself). */
#include "sys.h"

void exit(int status) {
    poly_syscall3(SYS_exit, status, 0, 0);
    for (;;) {
    }
}

long poly_write(int fd, const char *buf, long len) {
    return poly_syscall3(SYS_write, fd, (long)buf, len);
}

void _tlv_bootstrap(void) {}

void dyld_stub_binder(void) __asm__("dyld_stub_binder");
void dyld_stub_binder(void) {}
