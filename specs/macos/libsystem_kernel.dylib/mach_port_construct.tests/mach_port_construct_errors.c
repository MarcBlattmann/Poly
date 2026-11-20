/* poly-mac: libsystem
 * What mach_port_construct refuses: a task port that is not valid is MACH_SEND_INVALID_DEST; an options pointer or an out
 * pointer that is not readable / writable is KERN_MEMORY_ERROR (10), and no port is left behind that the caller could name. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_options_t o;
    memset(&o, 0, sizeof o);
    mach_port_t p = 0x7777;
    check("MACH_PORT_NULL as the task: MACH_SEND_INVALID_DEST", mach_port_construct(MACH_PORT_NULL, &o, 0, &p) == MACH_SEND_INVALID_DEST, 0);
    check("a name that is no task: MACH_SEND_INVALID_DEST", mach_port_construct(0x4321, &o, 0, &p) == MACH_SEND_INVALID_DEST, 0);
    check("the out name is left alone", p == 0x7777, p);
    kern_return_t kr = mach_port_construct(mach_task_self(), NULL, 0, &p);
    check("NULL options: KERN_MEMORY_ERROR (10)", kr == 10, kr);
    kr = mach_port_construct(mach_task_self(), &o, 0, NULL);
    check("NULL out pointer: KERN_MEMORY_ERROR (10)", kr == 10, kr);
    mach_port_t recv = new_receive();
    kr = mach_port_construct(recv, &o, 0, &p);
    check("a receive right as the task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
