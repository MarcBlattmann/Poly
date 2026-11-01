/* sethostname without CAP_SYS_ADMIN over the UTS namespace is EPERM,
 * and the permission check comes first: a too long or negative length
 * and an unreadable name are EPERM too. The name stays as it was.
 * Run as a normal user (root first becomes uid/gid 65534 without groups
 * in a child, which drops its capabilities), so the output is the same
 * for every caller.
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_sethostname 170

static struct a_utsname before, u;

static int child(void) {
    if (syscall0(SYS_geteuid) == 0) {
        if (syscall2(SYS_setgroups, 0, 0) || syscall3(SYS_setresgid, 65534, 65534, 65534) ||
            syscall3(SYS_setresuid, 65534, 65534, 65534))
            return 2;
    }
    expect("valid_name_eperm", syscall2(SYS_sethostname, "nope", 4), -EPERM);
    expect("len0_eperm", syscall2(SYS_sethostname, "", 0), -EPERM);
    expect("len_65_eperm", syscall2(SYS_sethostname, "x", 65), -EPERM);
    expect("len_minus1_eperm", syscall2(SYS_sethostname, "x", -1), -EPERM);
    expect("null_eperm", syscall2(SYS_sethostname, 0, 3), -EPERM);
    syscall1(SYS_uname, &u);
    check("name_unchanged", str_eq(u.nodename, before.nodename), 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    syscall1(SYS_uname, &before);
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
