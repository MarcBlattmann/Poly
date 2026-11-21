/* poly-mac: libsystem
 * What semaphore_destroy refuses: an invalid task (MACH_PORT_NULL, a name that is nothing): MACH_SEND_INVALID_DEST (0x10000003); a semaphore name that
 * is nothing: MACH_SEND_INVALID_RIGHT (0x1000000a); MACH_PORT_NULL, the task port or the host port as the semaphore: KERN_INVALID_ARGUMENT (4). A refused
 * call leaves the semaphore as it was. */
#include "poly_mexc.h"
#include <mach/semaphore.h>
#include <mach/sync_policy.h>
int test_main(void) {
    mach_port_t t = mach_task_self();
    semaphore_t s = 0;
    semaphore_create(t, &s, SYNC_POLICY_FIFO, 1);
    check("MACH_PORT_NULL as the task: MACH_SEND_INVALID_DEST", semaphore_destroy(MACH_PORT_NULL, s) == MACH_SEND_INVALID_DEST, 0);
    check("a name that is nothing as the task: MACH_SEND_INVALID_DEST", semaphore_destroy(0x1234, s) == MACH_SEND_INVALID_DEST, 0);
    check("a semaphore name that is nothing: MACH_SEND_INVALID_RIGHT", semaphore_destroy(t, 0x1234) == MACH_SEND_INVALID_RIGHT, 0);
    check("MACH_PORT_NULL as the semaphore: KERN_INVALID_ARGUMENT", semaphore_destroy(t, MACH_PORT_NULL) == KERN_INVALID_ARGUMENT, 0);
    check("the task port as the semaphore: KERN_INVALID_ARGUMENT", semaphore_destroy(t, t) == KERN_INVALID_ARGUMENT, 0);
    check("the host port as the semaphore: KERN_INVALID_ARGUMENT", semaphore_destroy(t, mach_host_self()) == KERN_INVALID_ARGUMENT, 0);
    check("the semaphore is untouched: its value 1 lets a wait through", semaphore_wait(s) == KERN_SUCCESS, 0);
    return finish();
}
