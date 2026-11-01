/* unshare(flags) flag validation, in a single-threaded child: 0 and each
 * of the non-namespace flags CLONE_VM, CLONE_FS, CLONE_FILES,
 * CLONE_SIGHAND, CLONE_THREAD and CLONE_SYSVSEM (and all of them at
 * once) succeed with nothing shared, for any caller. Every other bit
 * that is not a namespace flag is EINVAL: CSIGNAL bits 0..6, bits 12..15,
 * 19..24, CLONE_IO (bit 31) and every bit 32..63. An invalid bit makes
 * the call EINVAL even together with a namespace flag the caller may not
 * use (flags are checked before privileges), and nothing is unshared.
 */
#include "a_sys.h"

#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWTIME 0x80
#define CLONE_NEWNS 0x20000
#define CLONE_NEWCGROUP 0x02000000
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
#define CLONE_NEWUSER 0x10000000
#define CLONE_NEWPID 0x20000000
#define CLONE_NEWNET 0x40000000
#define CLONE_IO 0x80000000UL

#define NS_FLAGS                                                                                                    \
    (CLONE_NEWTIME | CLONE_NEWNS | CLONE_NEWCGROUP | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWUSER | CLONE_NEWPID | \
     CLONE_NEWNET)
#define PLAIN_OK (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static int child(void) {
    expect("zero", syscall1(SYS_unshare, 0), 0);
    expect("vm", syscall1(SYS_unshare, CLONE_VM), 0);
    expect("fs", syscall1(SYS_unshare, CLONE_FS), 0);
    expect("files", syscall1(SYS_unshare, CLONE_FILES), 0);
    expect("sighand", syscall1(SYS_unshare, CLONE_SIGHAND), 0);
    expect("thread", syscall1(SYS_unshare, CLONE_THREAD), 0);
    expect("sysvsem", syscall1(SYS_unshare, CLONE_SYSVSEM), 0);
    expect("all_plain", syscall1(SYS_unshare, PLAIN_OK), 0);

    int bad = 0, first_wrong = -1;
    for (int b = 0; b < 64; b++) {
        u64 f = 1UL << b;
        if (f & (NS_FLAGS | PLAIN_OK)) continue;
        bad++;
        if (syscall1(SYS_unshare, f) != -EINVAL && first_wrong < 0) first_wrong = b;
    }
    expect("other_bits_einval", first_wrong, -1);
    expect("other_bit_count", bad, 50);
    expect("clone_io_einval", syscall1(SYS_unshare, CLONE_IO), -EINVAL);
    expect("csignal_einval", syscall1(SYS_unshare, SIGCHLD), -EINVAL);
    expect("vfork_einval", syscall1(SYS_unshare, CLONE_VFORK), -EINVAL);
    expect("settls_einval", syscall1(SYS_unshare, CLONE_SETTLS), -EINVAL);
    expect("bit32_einval", syscall1(SYS_unshare, 1UL << 32), -EINVAL);
    expect("files_with_bit63_einval", syscall1(SYS_unshare, CLONE_FILES | (1UL << 63)), -EINVAL);
    expect("newuts_with_bad_bit_einval", syscall1(SYS_unshare, CLONE_NEWUTS | 1), -EINVAL);
    expect("newpid_with_bit40_einval", syscall1(SYS_unshare, CLONE_NEWPID | (1UL << 40)), -EINVAL);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
