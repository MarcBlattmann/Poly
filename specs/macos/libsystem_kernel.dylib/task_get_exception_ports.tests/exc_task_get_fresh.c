/* poly-mac: libsystem
 * task_get_exception_ports(task, mask, ...) in a process that has set nothing: every exception of the mask (1 to 9: access to RPC alert) is reported with a
 * null port, behavior 0 and flavor 0, the masks of the entries together are the mask asked for and do not overlap; for the thread of a
 * process (thread_get_exception_ports) a fresh thread has no entries at all. */
#include "poly_mexc.h"
int test_main(void) {
    exception_mask_t all = EXC_MASK_CLASSIC;
    exc_entry_t e[14];
    int n = exc_get(mach_task_self(), 0, all, e);
    check("the call succeeds and reports at least one entry", n >= 1, n);
    exception_mask_t union_mask = 0, overlap = 0;
    int all_null = 1, plain = 1;
    for (int i = 0; i < n; i++) {
        overlap |= union_mask & e[i].mask;
        union_mask |= e[i].mask;
        all_null &= e[i].port == MACH_PORT_NULL;
        plain &= e[i].behavior == 0 && e[i].flavor == 0;
    }
    check("the entries together are the mask asked for", union_mask == all, union_mask);
    check("without overlap", overlap == 0, overlap);
    check("every port is null", all_null, 0);
    check("with behavior 0 and flavor 0", plain, 0);
    value("number of entries", n);
    exc_entry_t t[14];
    int tn = exc_get(mach_thread_self(), 1, EXC_MASK_ALL, t);
    check("thread_get_exception_ports of a fresh thread succeeds with no entries", tn == 0, tn);
    int one = exc_get(mach_task_self(), 0, EXC_MASK_BAD_ACCESS, e);
    check("asking for one exception gives one entry with that mask", one == 1 && e[0].mask == EXC_MASK_BAD_ACCESS && e[0].port == MACH_PORT_NULL, one);
    return finish();
}
