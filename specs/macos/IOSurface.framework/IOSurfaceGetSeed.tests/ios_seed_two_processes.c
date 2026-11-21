/* poly-mac: iosurface
 * The seed is the surface's, shared by the processes that have it: a child that looks the surface up through its Mach port and locks it read-write raises the
 * seed the parent sees; the seed the child's lock stores is the parent's seed plus one. */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        check("child: the surface was found", s != NULL, 0);
        check("child: its seed is the parent's, 1", s && IOSurfaceGetSeed(s) == 1, s ? (long long)IOSurfaceGetSeed(s) : -1);
        uint32_t seed = 0;
        IOSurfaceLock(s, 0, &seed);
        IOSurfaceUnlock(s, 0, NULL);
        check("child: its read-write lock stores 2", seed == 2, seed);
        return finish();
    }
    char name[100];
    ios_service_name(name, sizeof name, "seed");
    IOSurfaceRef s = ios_make(32, 32, 4);
    IOSurfaceLock(s, 0, NULL);
    IOSurfaceUnlock(s, 0, NULL);
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    kern_return_t sk = kr ? kr : ios_send_port(cp, IOSurfaceCreateMachPort(s), 1);
    int status = ios_wait(pid);
    check("the parent sent the port", sk == KERN_SUCCESS, sk);
    check("the child ended with status 0", status == 0, status);
    check("the parent now sees seed 2", IOSurfaceGetSeed(s) == 2, IOSurfaceGetSeed(s));
    return finish();
}
