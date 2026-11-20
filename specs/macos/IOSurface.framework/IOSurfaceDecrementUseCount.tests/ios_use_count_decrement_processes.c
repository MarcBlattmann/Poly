/* poly-mac: iosurface
 * A decrement is against the calling process's own count: a child with a count of its own does not change the parent's, and a child that decrements while its own count is 0
 * does not take the parent's count down either. */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        if (!s) return 4;
        IOSurfaceDecrementUseCount(s);
        IOSurfaceDecrementUseCount(s);
        check("child: decrements at 0 leave its count 0", IOSurfaceGetUseCount(s) == 0, IOSurfaceGetUseCount(s));
        check("child: the parent's count still holds the surface in use", IOSurfaceIsInUse(s), 0);
        return finish();
    }
    char name[100];
    ios_service_name(name, sizeof name, "decproc");
    IOSurfaceRef s = ios_make(32, 32, 4);
    IOSurfaceIncrementUseCount(s);
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    if (!kr) ios_send_port(cp, IOSurfaceCreateMachPort(s), 1);
    int status = ios_wait(pid);
    check("the child ended with status 0", status == 0, status);
    check("the parent's count is still 1", IOSurfaceGetUseCount(s) == 1 && IOSurfaceIsInUse(s), IOSurfaceGetUseCount(s));
    return finish();
}
