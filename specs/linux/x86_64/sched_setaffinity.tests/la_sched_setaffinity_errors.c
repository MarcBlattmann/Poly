/* sched_setaffinity errors, in order: the mask is read first (an
 * unreadable mask with len > 0 is EFAULT, even for a pid that does not
 * exist); then the pid (no such thread: ESRCH, even when the mask is
 * empty); then the mask must contain at least one existing CPU the
 * thread may use: an all-zero mask, len 0, or only the bit of a CPU that
 * does not exist (1023) is EINVAL. A failed call leaves the affinity as
 * it was. len is a 32-bit unsigned value.
 * poly-diff: syscalls=names (sched_getaffinity returns the kernel's mask size, which depends on the machine)
 */
#include "a_sys.h"

#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 orig[16], got[16], zero[16], far[16];

static i64 set(i64 pid, u64 len, const void *m) { return syscall3(SYS_sched_setaffinity, pid, len, m); }

int test_main(void) {
    i64 n = syscall3(SYS_sched_getaffinity, 0, sizeof orig, orig);
    far[15] = 1UL << 63;

    expect("null_mask_efault", set(0, 128, 0), -EFAULT);
    expect("efault_beats_esrch", set(0x3ffffff0, 128, 0), -EFAULT);
    expect("unused_pid_esrch", set(0x3ffffff0, 128, orig), -ESRCH);
    expect("minus1_esrch", set(-1, 128, orig), -ESRCH);
    expect("esrch_beats_empty", set(0x3ffffff0, 128, zero), -ESRCH);
    expect("len_0_null_mask_esrch", set(0x3ffffff0, 0, 0), -ESRCH);

    expect("empty_mask_einval", set(0, 128, zero), -EINVAL);
    expect("len_0_einval", set(0, 0, orig), -EINVAL);
    expect("len_0_null_mask_einval", set(0, 0, 0), -EINVAL);
    expect("nonexistent_cpu_only_einval", set(0, 128, far), -EINVAL);
    memset(got, 0, sizeof got);
    syscall3(SYS_sched_getaffinity, 0, sizeof got, got);
    check("failed_calls_change_nothing", mem_eq(got, orig, (size_t)n), 0);
    expect("len_upper_bits_ignored", set(0, (1UL << 32) | 128, orig), 0);
    return finish();
}
