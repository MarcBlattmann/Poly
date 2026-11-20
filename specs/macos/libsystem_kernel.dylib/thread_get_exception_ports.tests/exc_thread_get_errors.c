/* poly-mac: libsystem
 * thread_get_exception_ports with a mask of 0: success, no entries; mask 0x80000000: KERN_INVALID_ARGUMENT (4); the task port instead of a thread
 * port: KERN_INVALID_ARGUMENT; an invalid thread (MACH_PORT_NULL, a name that is nothing): MACH_SEND_INVALID_DEST (0x10000003). */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t th = mach_thread_self();
    exc_entry_t e[14];
    check("mask 0: success with no entries", exc_get(th, 1, 0, e) == 0, 0);
    check("mask 0x80000000: KERN_INVALID_ARGUMENT", exc_get(th, 1, 0x80000000, e) == -1 - KERN_INVALID_ARGUMENT, 0);
    check("MACH_PORT_NULL as the thread: MACH_SEND_INVALID_DEST", exc_get(MACH_PORT_NULL, 1, EXC_MASK_ALL, e) == -1 - MACH_SEND_INVALID_DEST, 0);
    check("a name that is nothing as the thread: MACH_SEND_INVALID_DEST", exc_get(0x1234, 1, EXC_MASK_ALL, e) == -1 - MACH_SEND_INVALID_DEST, 0);
    check("the task port as the thread: KERN_INVALID_ARGUMENT", exc_get(mach_task_self(), 1, EXC_MASK_ALL, e) == -1 - KERN_INVALID_ARGUMENT, 0);
    return finish();
}
