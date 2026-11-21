/* poly-mac: iosurface
 * The port can be sent in a message to another process (a port descriptor, COPY_SEND): the child's IOSurfaceLookupFromMachPort gives the same surface, its memory is the same
 * memory (a byte the child writes under a lock is seen by the parent), a surface that is not global is found this way, and the child's sizes and format are the parent's. */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        check("child: found", s != NULL, 0);
        check("child: sizes and format are the parent's", s && IOSurfaceGetWidth(s) == 100 && IOSurfaceGetHeight(s) == 50 && IOSurfaceGetBytesPerRow(s) == 512 && IOSurfaceGetPixelFormat(s) == 'BGRA', 0);
        IOSurfaceLock(s, 0, NULL);
        unsigned char *b = IOSurfaceGetBaseAddress(s);
        check("child: sees the parent's bytes", b[0] == 0x11 && b[100] == 0x22, 0);
        b[1] = 0xaa;
        IOSurfaceUnlock(s, 0, NULL);
        return finish();
    }
    char name[100];
    ios_service_name(name, sizeof name, "port2");
    IOSurfaceRef s = ios_make(100, 50, 4);
    IOSurfaceLock(s, 0, NULL);
    unsigned char *b = IOSurfaceGetBaseAddress(s);
    b[0] = 0x11;
    b[100] = 0x22;
    IOSurfaceUnlock(s, 0, NULL);
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    kern_return_t sk = kr ? kr : ios_send_port(cp, IOSurfaceCreateMachPort(s), 1);
    int status = ios_wait(pid);
    check("the port was sent", sk == KERN_SUCCESS, sk);
    check("the child ended with status 0", status == 0, status);
    IOSurfaceLock(s, kIOSurfaceLockReadOnly, NULL);
    check("the parent sees the byte the child wrote", ((unsigned char *)IOSurfaceGetBaseAddress(s))[1] == 0xaa, 0);
    IOSurfaceUnlock(s, kIOSurfaceLockReadOnly, NULL);
    return finish();
}
