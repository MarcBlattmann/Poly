/* sched_getattr errors and their order: a NULL attr, a negative pid or
 * flags other than 0 are EINVAL first; then a size outside 48..4096 is
 * EINVAL; then a pid with no task is ESRCH; then an unwritable buffer is
 * EFAULT. flags is 32 bits (upper half ignored).
 */
#include "a_sys.h"

#define SYS_sched_getattr 315
#define NO_PID 0x7fffffff

static i64 g(i64 pid, void *buf, u64 size, u64 flags) { return syscall4(SYS_sched_getattr, pid, buf, size, flags); }

int test_main(void) {
    static unsigned char buf[64];
    expect("ok", g(0, buf, 56, 0), 0);
    expect("null_einval", g(0, 0, 56, 0), -EINVAL);
    expect("flags1_einval", g(0, buf, 56, 1), -EINVAL);
    expect("flags_high_bit_einval", g(0, buf, 56, 0x80000000UL), -EINVAL);
    expect("flags_upper_half_ignored", g(0, buf, 56, 1UL << 32), 0);
    expect("pid_minus1_einval", g(-1, buf, 56, 0), -EINVAL);
    expect("no_pid_esrch", g(NO_PID, buf, 56, 0), -ESRCH);
    expect("bad_buffer_efault", g(0, (void *)16, 56, 0), -EFAULT);

    expect("null_before_flags", g(0, 0, 56, 1), -EINVAL);
    expect("null_before_pid", g(NO_PID, 0, 56, 0), -EINVAL);
    expect("negative_pid_before_size", g(-1, buf, 8, 0), -EINVAL);
    expect("size_before_lookup", g(NO_PID, buf, 8, 0), -EINVAL);
    expect("size_big_before_lookup", g(NO_PID, buf, 8192, 0), -EINVAL);
    expect("lookup_before_fault", g(NO_PID, (void *)16, 56, 0), -ESRCH);
    expect("size_before_fault", g(0, (void *)16, 4, 0), -EINVAL);
    return finish();
}
