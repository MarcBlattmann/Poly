/* poly-mac: iosurface
 * The use count is the process's own: a child that increments the surface (found through its Mach port) has its own count 1 while the parent's stays 0; the parent's
 * increments do not show in the child either; the surface is in use as long as either has a count. */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        if (!s) return 4;
        check("child: its use count starts at 0 (the parent's increments are not its own)", IOSurfaceGetUseCount(s) == 0, IOSurfaceGetUseCount(s));
        check("child: but the surface is in use (the parent holds a count)", IOSurfaceIsInUse(s), 0);
        IOSurfaceIncrementUseCount(s);
        check("child: after its increment its count is 1", IOSurfaceGetUseCount(s) == 1, IOSurfaceGetUseCount(s));
        return finish();
    }
    char name[100];
    ios_service_name(name, sizeof name, "incproc");
    IOSurfaceRef s = ios_make(32, 32, 4);
    IOSurfaceIncrementUseCount(s);
    IOSurfaceIncrementUseCount(s);
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    if (!kr) ios_send_port(cp, IOSurfaceCreateMachPort(s), 1);
    int status = ios_wait(pid);
    check("the child ended with status 0", status == 0, status);
    check("the parent's count is still 2", IOSurfaceGetUseCount(s) == 2, IOSurfaceGetUseCount(s));
    return finish();
}
