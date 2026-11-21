/* poly-mac: libsystem
 * semaphore_create with a task that is not a task port: MACH_PORT_NULL or a name that is nothing is MACH_SEND_INVALID_DEST (0x10000003); the host port
 * is KERN_INVALID_ARGUMENT. The out name is left alone by a failed call. */
#include "poly_mexc.h"
#include <mach/semaphore.h>
#include <mach/sync_policy.h>
int test_main(void) {
    semaphore_t s = 0x7777;
    check("MACH_PORT_NULL as the task: MACH_SEND_INVALID_DEST", semaphore_create(MACH_PORT_NULL, &s, SYNC_POLICY_FIFO, 0) == MACH_SEND_INVALID_DEST, 0);
    check("a name that is nothing as the task: MACH_SEND_INVALID_DEST", semaphore_create(0x1234, &s, SYNC_POLICY_FIFO, 0) == MACH_SEND_INVALID_DEST, 0);
    check("the host port as the task: KERN_INVALID_ARGUMENT", semaphore_create(mach_host_self(), &s, SYNC_POLICY_FIFO, 0) == KERN_INVALID_ARGUMENT, 0);
    check("the out name is left alone", s == 0x7777, s);
    return finish();
}
