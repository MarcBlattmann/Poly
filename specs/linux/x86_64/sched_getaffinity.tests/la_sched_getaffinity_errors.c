/* sched_getaffinity errors, in order: len not a multiple of 8 (or 0,
 * or, on machines with more possible CPUs, too small for the kernel's
 * mask) is EINVAL, checked before the pid; a pid with no such thread
 * (an unused number, -1) is ESRCH, checked before the mask pointer; an
 * unwritable mask (NULL, read-only memory) is EFAULT.
 */
#include "a_sys.h"

#define SYS_sched_getaffinity 204

static u64 mask[16];

static i64 get(i64 pid, u64 len, void *m) { return syscall3(SYS_sched_getaffinity, pid, len, m); }

int test_main(void) {
    expect("len_0_einval", get(0, 0, mask), -EINVAL);
    expect("len_4_einval", get(0, 4, mask), -EINVAL);
    expect("len_7_einval", get(0, 7, mask), -EINVAL);
    expect("len_9_einval", get(0, 9, mask), -EINVAL);
    expect("len_12_einval", get(0, 12, mask), -EINVAL);
    expect("len_127_einval", get(0, 127, mask), -EINVAL);
    expect("bad_len_beats_esrch", get(0x3ffffff0, 7, mask), -EINVAL);
    expect("bad_len_beats_efault", get(0, 7, 0), -EINVAL);

    expect("unused_pid_esrch", get(0x3ffffff0, 128, mask), -ESRCH);
    expect("minus1_esrch", get(-1, 128, mask), -ESRCH);
    expect("esrch_beats_efault", get(0x3ffffff0, 128, 0), -ESRCH);

    expect("null_mask_efault", get(0, 128, 0), -EFAULT);
    void *ro = (void *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("read_only_mask_efault", get(0, 128, ro), -EFAULT);
    return finish();
}
