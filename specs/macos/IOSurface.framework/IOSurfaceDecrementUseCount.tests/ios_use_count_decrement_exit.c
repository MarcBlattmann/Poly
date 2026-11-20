/* poly-mac: iosurface
 * A process that ends with its counts still up drops them: a child that increments the surface twice and exits without decrementing leaves the surface not in use
 * (the parent's count is 0). */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        if (!s) return 4;
        IOSurfaceIncrementUseCount(s);
        IOSurfaceIncrementUseCount(s);
        return 0;
    }
    char name[100];
    ios_service_name(name, sizeof name, "decexit");
    IOSurfaceRef s = ios_make(32, 32, 4);
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    mach_port_t mp = IOSurfaceCreateMachPort(s);
    if (!kr) ios_send_port(cp, mp, 1);
    int status = ios_wait(pid);
    check("the child ended with status 0", status == 0, status);
    mach_port_deallocate(mach_task_self(), mp);
    int in_use = 1;
    for (int i = 0; i < 40 && in_use; i++) {
        in_use = IOSurfaceIsInUse(s);
        if (in_use) usleep(50000);
    }
    check("shortly after it exited the surface is not in use", !in_use, 0);
    return finish();
}
