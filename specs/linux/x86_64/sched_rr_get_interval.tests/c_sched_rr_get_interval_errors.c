/* sched_rr_get_interval errors and their order: a negative pid is EINVAL
 * first (also with a NULL pointer); then the pid is looked up (ESRCH,
 * also with a NULL or bad pointer); then the result is written: a NULL
 * pointer is EFAULT here (not EINVAL), as are a low address, a
 * non-canonical address, read-only memory and a struct that runs into
 * an unmapped page. Nothing is written on error.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_rr_get_interval 148
/* An id no task has: above PID_MAX_LIMIT (linux/threads.h). */
#define NO_ID 0x3ffffff0

static const i64 ro_ts[2] = { 5, 6 };

static i64 rr(i64 pid, void *ts) { return syscall2(SYS_sched_rr_get_interval, pid, ts); }

int test_main(void) {
    i64 ts[2] = { 11, 22 };
    i64 r = rr(-1, ts);
    check("negative_pid_einval", r == -EINVAL, r);
    check("einval_nothing_written", ts[0] == 11 && ts[1] == 22, ts[0]);
    r = rr(-1, 0);
    check("negative_pid_null_einval", r == -EINVAL, r);
    r = rr(0xffffffffL, ts);
    check("low_half_minus1_einval", r == -EINVAL, r);
    r = rr(NO_ID, ts);
    check("no_such_pid_esrch", r == -ESRCH, r);
    check("esrch_nothing_written", ts[0] == 11 && ts[1] == 22, ts[0]);
    r = rr(NO_ID, 0);
    check("esrch_beats_null", r == -ESRCH, r);
    r = rr(NO_ID, (void *)1);
    check("esrch_beats_efault", r == -ESRCH, r);
    r = rr(0, 0);
    check("null_efault", r == -EFAULT, r);
    r = rr(0, (void *)1);
    check("low_address_efault", r == -EFAULT, r);
    r = rr(0, (void *)0x800000000000UL);
    check("noncanonical_efault", r == -EFAULT, r);
    r = rr(0, (void *)ro_ts);
    check("read_only_efault", r == -EFAULT, r);
    check("read_only_unchanged", ro_ts[0] == 5 && ro_ts[1] == 6, ro_ts[0]);

    /* A struct whose second half lies in an unmapped page. */
    i64 m = syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    syscall2(SYS_munmap, m + 4096, 4096);
    r = rr(0, (void *)(m + 4096 - 8));
    check("straddling_efault", r == -EFAULT, r);
    r = rr(0, (void *)(m + 4096 - 16));
    check("last_16_bytes_ok", r == 0, r);
    return finish();
}
