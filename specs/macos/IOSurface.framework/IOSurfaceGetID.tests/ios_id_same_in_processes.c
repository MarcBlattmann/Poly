/* poly-mac: iosurface
 * The ID of a surface is the same in the process that made it and in a process that got the surface through its Mach port (IOSurfaceGetID in the child is the parent's). */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        if (!s) return 4;
        return (int)(IOSurfaceGetID(s) & 0x7f) + 10; /* the low bits of the ID, for the parent to compare */
    }
    char name[100];
    ios_service_name(name, sizeof name, "idsame");
    IOSurfaceRef s = ios_make(32, 32, 4);
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    if (!kr) ios_send_port(cp, IOSurfaceCreateMachPort(s), 1);
    int status = ios_wait(pid);
    check("the child found the surface and answered", status >= 10 && status < 138, status);
    check("the low bits of the ID the child reports are the parent's", status == (int)(IOSurfaceGetID(s) & 0x7f) + 10, status);
    return finish();
}
