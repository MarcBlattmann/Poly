/* poly-mac: iosurface
 * IOSurfaceLookupFromMachPort(port) in the process that made the port gives a new reference to the same surface (not the same object; same ID, sizes, base
 * address, bytes); MACH_PORT_NULL, a name that is nothing (0x1234), a dead name and an ordinary port (a receive right with a send right, made for the test) give NULL. */
#include "poly_miosurface.h"
int test_main(void) {
    IOSurfaceRef s = ios_make(64, 64, 4);
    mach_port_t mp = IOSurfaceCreateMachPort(s);
    IOSurfaceRef r = IOSurfaceLookupFromMachPort(mp);
    check("found", r != NULL, 0);
    check("another object for the same surface", r != s && IOSurfaceGetID(r) == IOSurfaceGetID(s), 0);
    check("same sizes and base address", IOSurfaceGetWidth(r) == 64 && IOSurfaceGetBytesPerRow(r) == IOSurfaceGetBytesPerRow(s) && IOSurfaceGetBaseAddress(r) == IOSurfaceGetBaseAddress(s), 0);
    check("MACH_PORT_NULL: NULL", IOSurfaceLookupFromMachPort(MACH_PORT_NULL) == NULL, 0);
    check("0x1234: NULL", IOSurfaceLookupFromMachPort(0x1234) == NULL, 0);
    mach_port_t rp = MACH_PORT_NULL;
    mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &rp);
    mach_port_insert_right(mach_task_self(), rp, rp, MACH_MSG_TYPE_MAKE_SEND);
    check("an ordinary port: NULL", IOSurfaceLookupFromMachPort(rp) == NULL, 0);
    mach_port_t dead = MACH_PORT_NULL;
    mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &dead);
    mach_port_insert_right(mach_task_self(), dead, dead, MACH_MSG_TYPE_MAKE_SEND);
    mach_port_mod_refs(mach_task_self(), dead, MACH_PORT_RIGHT_RECEIVE, -1);
    check("a dead name: NULL", IOSurfaceLookupFromMachPort(dead) == NULL, 0);
    check("MACH_PORT_DEAD: NULL", IOSurfaceLookupFromMachPort(MACH_PORT_DEAD) == NULL, 0);
    return finish();
}
