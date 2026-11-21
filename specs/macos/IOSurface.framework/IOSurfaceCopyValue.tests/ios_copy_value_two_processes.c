/* poly-mac: iosurface
 * Attachments are the surface's: set in one process, copied in the other, in both directions, and the values of different keys do not disturb each other; a value
 * the parent removes is gone for the child as well. */
#include "poly_miosurface.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t rp = MACH_PORT_NULL, sp = MACH_PORT_NULL;
        if (bootstrap_check_in(bootstrap_port, (char *)ios_child_arg(), &rp)) return 2;
        if (ios_receive_port(rp, &sp)) return 3;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(sp);
        if (!s) return 4;
        check("child: the parent's string", ios_str_eq(IOSurfaceCopyValue(s, CFSTR("poly.a")), "one"), 0);
        check("child: the parent's number", ios_num(IOSurfaceCopyValue(s, CFSTR("poly.n"))) == 7, 0);
        IOSurfaceSetValue(s, CFSTR("poly.b"), CFSTR("two"));
        usleep(800000);
        check("child: a key the parent removed is gone", IOSurfaceCopyValue(s, CFSTR("poly.a")) == NULL, 0);
        return finish();
    }
    char name[100];
    ios_service_name(name, sizeof name, "attach");
    IOSurfaceRef s = ios_make(64, 64, 4);
    IOSurfaceSetValue(s, CFSTR("poly.a"), CFSTR("one"));
    int seven = 7;
    IOSurfaceSetValue(s, CFSTR("poly.n"), CFNumberCreate(NULL, kCFNumberIntType, &seven));
    pid_t pid = ios_start_self("child", name);
    mach_port_t cp = MACH_PORT_NULL;
    kern_return_t kr = ios_wait_service(name, &cp);
    if (!kr) ios_send_port(cp, IOSurfaceCreateMachPort(s), 1);
    usleep(400000);
    check("the parent sees the child's string", ios_str_eq(IOSurfaceCopyValue(s, CFSTR("poly.b")), "two"), 0);
    IOSurfaceRemoveValue(s, CFSTR("poly.a"));
    int status = ios_wait(pid);
    check("the child ended with status 0", status == 0, status);
    return finish();
}
