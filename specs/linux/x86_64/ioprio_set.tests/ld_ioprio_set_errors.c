/* ioprio_set errors and their order. The priority is checked first:
 * an invalid class or level is EINVAL whatever `which` and `who` are, and
 * the real-time class (1) without CAP_SYS_ADMIN/CAP_SYS_NICE is EPERM
 * before anything else. Then `which` (only 1 process, 2 process group,
 * 3 user; else EINVAL), then the target: no such process, group or user
 * with processes is ESRCH. A failed call changes nothing.
 */
#include "a_sys.h"

#define SYS_ioprio_set 251
#define SYS_ioprio_get 252
/* linux/ioprio.h */
#define IOPRIO_WHO_PROCESS 1
#define IOPRIO_WHO_PGRP 2
#define IOPRIO_WHO_USER 3
#define PRIO(c, l) (((c) << 13) | (l))
#define NO_ID 0x7fffffff /* above any pid_max: no such process, group or user */

static i64 set(i64 which, i64 who, i64 prio) { return syscall3(SYS_ioprio_set, which, who, prio); }

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    expect("start_be4", set(IOPRIO_WHO_PROCESS, 0, PRIO(2, 4)), 0);

    expect("which0_einval", set(0, 0, PRIO(2, 1)), -EINVAL);
    expect("which4_einval", set(4, 0, PRIO(2, 1)), -EINVAL);
    expect("which_minus1_einval", set(-1, 0, PRIO(2, 1)), -EINVAL);
    expect("no_process_esrch", set(IOPRIO_WHO_PROCESS, NO_ID, PRIO(2, 1)), -ESRCH);
    expect("pid_minus1_esrch", set(IOPRIO_WHO_PROCESS, -1, PRIO(2, 1)), -ESRCH);
    expect("no_pgrp_esrch", set(IOPRIO_WHO_PGRP, NO_ID, PRIO(2, 1)), -ESRCH);
    expect("no_user_esrch", set(IOPRIO_WHO_USER, NO_ID, PRIO(2, 1)), -ESRCH);

    /* The priority is checked before which and who. */
    expect("bad_prio_bad_which", set(9, 0, PRIO(5, 0)), -EINVAL);
    expect("bad_prio_no_process", set(IOPRIO_WHO_PROCESS, NO_ID, PRIO(5, 0)), -EINVAL);
    expect("none_level_no_process", set(IOPRIO_WHO_PROCESS, NO_ID, PRIO(0, 3)), -EINVAL);
    /* Real time: EPERM without privilege, before which and who. */
    expect("rt_no_process", set(IOPRIO_WHO_PROCESS, NO_ID, PRIO(1, 0)), root ? -ESRCH : -EPERM);
    expect("rt_bad_which", set(9, 0, PRIO(1, 0)), root ? -EINVAL : -EPERM);
    expect("rt_bad_level_order", set(IOPRIO_WHO_PROCESS, NO_ID, PRIO(1, 0) | (1 << 3)), root ? -ESRCH : -EPERM);

    expect("unchanged", syscall2(SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0), PRIO(2, 4));
    return finish();
}
