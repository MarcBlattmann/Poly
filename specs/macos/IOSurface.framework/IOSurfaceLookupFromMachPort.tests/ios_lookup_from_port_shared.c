/* poly-mac: iosurface
 * What two processes share through the port: the data, the seed (a lock in the child raises the one the parent sees), the attachments (IOSurfaceSetValue in one is
 * seen by IOSurfaceCopyValue in the other), and the surface itself (the use count is not: it is each process's own). */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        if (!s) return 4;
        CFStringRef k = ios_cfstr("poly.shared");
        CFTypeRef v = IOSurfaceCopyValue(s, k);
        check("child: sees the parent's attachment", ios_str_eq(v, "from the parent"), 0);
        IOSurfaceSetValue(s, ios_cfstr("poly.child"), CFSTR("from the child"));
        IOSurfaceIncrementUseCount(s);
        check("child: its own use count is 1", IOSurfaceGetUseCount(s) == 1, IOSurfaceGetUseCount(s));
        return finish();
    }
    char name[100];
    ios_service_name(name, sizeof name, "shared");
    IOSurfaceRef s = ios_make(64, 64, 4);
    IOSurfaceSetValue(s, ios_cfstr("poly.shared"), CFSTR("from the parent"));
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    if (!kr) ios_send_port(cp, IOSurfaceCreateMachPort(s), 1);
    int status = ios_wait(pid);
    check("the child ended with status 0", status == 0, status);
    CFTypeRef v = IOSurfaceCopyValue(s, ios_cfstr("poly.child"));
    check("the parent sees the child's attachment", ios_str_eq(v, "from the child"), 0);
    check("the parent's use count is its own: 0", IOSurfaceGetUseCount(s) == 0, IOSurfaceGetUseCount(s));
    return finish();
}
