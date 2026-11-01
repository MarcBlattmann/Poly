/* sched_getparam errors and their order: a NULL param or a negative pid
 * is EINVAL first (NULL with a pid no task has is still EINVAL); then
 * the pid is looked up (ESRCH, also with an unwritable param); then the
 * result is written (EFAULT for an unmapped address, a low address or
 * read-only memory). A failed call writes nothing.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143
/* An id no task has: above PID_MAX_LIMIT (linux/threads.h). */
#define NO_ID 0x3ffffff0

static const int ro_word = 1234;

static i64 getparam_raw(i64 pid, void *p) { return syscall2(SYS_sched_getparam, pid, p); }

int test_main(void) {
    int v = 55;
    i64 r = getparam_raw(0, 0);
    check("null_einval", r == -EINVAL, r);
    r = getparam_raw(-1, &v);
    check("negative_pid_einval", r == -EINVAL, r);
    check("negative_pid_nothing_written", v == 55, v);
    r = getparam_raw(-1, 0);
    check("negative_pid_null_einval", r == -EINVAL, r);
    r = getparam_raw(NO_ID, 0);
    check("null_beats_esrch", r == -EINVAL, r);
    r = getparam_raw(NO_ID, &v);
    check("no_such_pid_esrch", r == -ESRCH, r);
    check("esrch_nothing_written", v == 55, v);
    r = getparam_raw(NO_ID, (void *)1);
    check("esrch_beats_efault", r == -ESRCH, r);
    r = getparam_raw(0, (void *)1);
    check("low_address_efault", r == -EFAULT, r);
    r = getparam_raw(0, (void *)0x800000000000UL);
    check("noncanonical_efault", r == -EFAULT, r);
    r = getparam_raw(0, (void *)&ro_word);
    check("read_only_efault", r == -EFAULT, r);
    check("read_only_unchanged", ro_word == 1234, ro_word);
    r = getparam_raw(0xffffffffL, &v);
    check("low_half_minus1_einval", r == -EINVAL, r);
    return finish();
}
