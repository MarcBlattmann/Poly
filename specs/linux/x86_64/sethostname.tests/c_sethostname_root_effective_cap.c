/* It is the effective set that counts: with CAP_SYS_ADMIN removed from
 * the effective set only (still permitted), sethostname fails with EPERM,
 * even for a length over 64 or a NULL name, and changes nothing; once the
 * capability is made effective again the same call succeeds.
 * poly-diff: needs=root (only root may set the host name)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sethostname 170
#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWUTS 0x04000000

static i64 sethostname(const void *name, i64 len) { return syscall2(SYS_sethostname, name, len); }

static A_UNUSED void nodename(char out[65]) {
    struct a_utsname u;
    memset(&u, 0x55, sizeof u);
    syscall1(SYS_uname, &u);
    memcpy(out, u.nodename, 65);
}

/* Run `body` in a child that has its own UTS namespace, so the real host
 * name is never touched. The child's output is the test's output. If the
 * namespace cannot be made, the child reports that and never calls
 * sethostname. */
static int in_new_uts(int (*body)(void)) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i64 r = syscall1(SYS_unshare, CLONE_NEWUTS);
        expect("unshare_newuts", r, 0);
        if (r != 0) syscall1(SYS_exit_group, finish());
        syscall1(SYS_exit_group, body());
    }
    i32 status = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return 1;
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 1;
}

/* asm/unistd_64.h */
#define SYS_capget 125
#define SYS_capset 126
/* linux/capability.h */
#define CAP_SYS_ADMIN 21
struct cap_header {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

int body(void) {
    char n[65];
    expect("set_known", sethostname("known", 5), 0);
    struct cap_header h = { 0x20080522, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    expect("capget", syscall2(SYS_capget, &h, d), 0);
    check("has_sys_admin", (d[0].permitted >> CAP_SYS_ADMIN) & 1, (i64)d[0].permitted);
    d[0].effective &= ~(1U << CAP_SYS_ADMIN);
    expect("drop_effective", syscall2(SYS_capset, &h, d), 0);
    expect("set_eperm", sethostname("other", 5), -EPERM);
    expect("len_65_eperm", sethostname("other", 65), -EPERM);
    expect("null_eperm", sethostname(0, 5), -EPERM);
    nodename(n);
    check("unchanged_after_eperm", str_eq(n, "known"), (i64)str_len(n));
    d[0].effective |= 1U << CAP_SYS_ADMIN;
    expect("raise_effective", syscall2(SYS_capset, &h, d), 0);
    expect("set_ok_again", sethostname("other", 5), 0);
    nodename(n);
    check("nodename_other", str_eq(n, "other"), (i64)str_len(n));
    return finish();
}

int test_main(void) { return in_new_uts(body); }
