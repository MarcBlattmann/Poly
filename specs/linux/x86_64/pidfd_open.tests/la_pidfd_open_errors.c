/* pidfd_open errors: flags other than PIDFD_NONBLOCK (O_NONBLOCK) and
 * PIDFD_THREAD (O_EXCL) are EINVAL (O_CLOEXEC, 1, O_RDWR, 0x80000000),
 * checked first; pid 0 or negative is EINVAL; a pid with no process is
 * ESRCH. pid is a 32-bit int and flags a 32-bit unsigned value: upper
 * halves are ignored.
 * poly-diff: syscalls=names (pidfd_open's pid argument is not masked in the trace, and pids differ between runs)
 */
#include "a_sys.h"

#define SYS_pidfd_open 434
#define PIDFD_NONBLOCK O_NONBLOCK
#define PIDFD_THREAD O_EXCL

static i64 po(i64 pid, u64 flags) {
    i64 fd = syscall2(SYS_pidfd_open, pid, flags);
    if (fd >= 0) a_close(fd);
    return fd;
}

int test_main(void) {
    i64 me = syscall0(SYS_getpid);
    expect("cloexec_flag_einval", po(me, O_CLOEXEC), -EINVAL);
    expect("flag_1_einval", po(me, 1), -EINVAL);
    expect("flag_rdwr_einval", po(me, O_RDWR), -EINVAL);
    expect("flag_bit31_einval", po(me, 0x80000000UL), -EINVAL);
    expect("nonblock_plus_bad_einval", po(me, PIDFD_NONBLOCK | 1), -EINVAL);
    expect("bad_flags_beat_esrch", po(0x3ffffff0, 1), -EINVAL);
    expect("pid_0_einval", po(0, 0), -EINVAL);
    expect("pid_minus1_einval", po(-1, 0), -EINVAL);
    expect("pid_negative_einval", po(-me, 0), -EINVAL);
    expect("unused_pid_esrch", po(0x3ffffff0, 0), -ESRCH);
    expect("nonblock_ok", po(me, PIDFD_NONBLOCK), 3);
    expect("thread_flag_ok", po(me, PIDFD_THREAD), 3);
    expect("both_ok", po(me, PIDFD_THREAD | PIDFD_NONBLOCK), 3);
    expect("pid_upper_bits_ignored", po((1L << 32) | me, 0), 3);
    expect("flags_upper_bits_ignored", po(me, 1UL << 32), 3);
    return finish();
}
