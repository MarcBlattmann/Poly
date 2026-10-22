/* A successful truncate by a caller without CAP_FSETID clears the
 * set-user-ID bit, and the set-group-ID bit when group execute is set
 * too; it does so also when the size does not change. A truncate that
 * fails (EFBIG over RLIMIT_FSIZE) leaves the mode alone. The other mode
 * bits are kept.
 * The checks run in a child; if the test runs as root, the child first
 * becomes uid/gid 65534 (losing CAP_FSETID; the files are then another
 * user's and the modes give "other" write permission).
 */
#include "a_sys.h"

#define D "/tmp/a_truncate_suid"
#define SIGXFSZ 25

/* asm/unistd_64.h */
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119

static u32 mode_of(const char *p) {
    struct a_stat st;
    a_stat_path(p, &st);
    return st.st_mode & 07777;
}

static int checks(void) {
    if (syscall0(SYS_geteuid) == 0) {
        check("drop_groups", syscall2(SYS_setgroups, 0, 0) == 0, 0);
        check("drop_gid", syscall3(SYS_setresgid, 65534, 65534, 65534) == 0, 0);
        check("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534) == 0, 0);
    } else {
        check("drop_groups", 1, 0);
        check("drop_gid", 1, 0);
        check("drop_uid", 1, 0);
    }
    expect("suid_shrink", syscall2(SYS_truncate, D "/suid", 1), 0);
    expect("suid_cleared", mode_of(D "/suid"), 0777);
    expect("suid_same_size", syscall2(SYS_truncate, D "/suid_same", 5), 0);
    expect("suid_same_cleared", mode_of(D "/suid_same"), 0757);
    expect("both_grow", syscall2(SYS_truncate, D "/both", 50), 0);
    expect("both_cleared", mode_of(D "/both"), 0777);
    expect("plain", syscall2(SYS_truncate, D "/plain", 0), 0);
    expect("plain_kept", mode_of(D "/plain"), 0666);
    struct a_sigaction sa = {SIG_IGN, 0, 0, 0};
    syscall4(SYS_rt_sigaction, SIGXFSZ, &sa, 0, 8);
    struct a_rlimit rl = {16, RLIM_INFINITY};
    syscall4(SYS_prlimit64, 0, RLIMIT_FSIZE, &rl, 0);
    i64 r = syscall2(SYS_truncate, D "/failed", 4096);
    rl.rlim_cur = RLIM_INFINITY;
    syscall4(SYS_prlimit64, 0, RLIMIT_FSIZE, &rl, 0);
    expect("failed_efbig", r, -EFBIG);
    expect("failed_mode_kept", mode_of(D "/failed"), 04777);
    return finish();
}

static int make(const char *p, u32 mode) {
    /* chmod after creating: the umask would clear bits otherwise. */
    return a_make_file(p, "hello", 0600) == 0 && syscall2(SYS_chmod, p, mode) == 0 && mode_of(p) == mode;
}

int test_main(void) {
    a_block_sigchld();
    check("setup_mkdir", a_mkdir(D, 0755) == 0, 0);
    check("setup_suid", make(D "/suid", 04777), 0);
    check("setup_suid_same", make(D "/suid_same", 04757), 0);
    check("setup_both", make(D "/both", 06777), 0);
    check("setup_plain", make(D "/plain", 0666), 0);
    check("setup_failed", make(D "/failed", 04777), 0);
    if (failed) return finish();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, checks());
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    a_unlink(D "/suid");
    a_unlink(D "/suid_same");
    a_unlink(D "/both");
    a_unlink(D "/plain");
    a_unlink(D "/failed");
    a_rmdir(D);
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 1;
}
