/* setpgid on a child from the parent; errors: no such process (ESRCH),
 * negative group (EINVAL), a group that does not exist in the session
 * (EPERM), a child that has called execve (EACCES). */
#include "a_sys.h"

static volatile i32 go, done;

static int child(void *arg) {
    (void)arg;
    while (!go) a_relax();
    return 0;
}

/* The checks run in a child that leads a new session and process group
 * (a_in_session), so they do not depend on how the test was started. */
static int body(void) {
    a_block_sigchld();
    long pid = a_spawn(0, &done, child, 0);
    expect("parent_moves_child", syscall2(SYS_setpgid, pid, pid), 0);
    expect("child_group_is_child", syscall1(SYS_getpgid, pid), pid);
    expect("parent_moves_child_back", syscall2(SYS_setpgid, pid, syscall0(SYS_getpgrp)), 0);
    expect("child_back", syscall1(SYS_getpgid, pid), syscall0(SYS_getpgrp));
    expect("negative_pgid_einval", syscall2(SYS_setpgid, pid, -1), -EINVAL);
    expect("missing_group_eperm", syscall2(SYS_setpgid, pid, 0x3ffffff0), -EPERM);
    expect("no_such_process_esrch", syscall2(SYS_setpgid, 0x3ffffff0, 0), -ESRCH);
    go = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    /* A child that has run execve can no longer be moved. */
    char *argv[] = {"x", "spin", 0};
    char *envp[] = {0};
    i64 v = syscall0(SYS_vfork);
    if (v == 0) {
        syscall3(SYS_execve, a_self_path(), argv, envp);
        syscall1(SYS_exit_group, 127);
    }
    expect("after_exec_eacces", syscall2(SYS_setpgid, v, v), -EACCES);
    syscall2(SYS_kill, v, SIGKILL);
    syscall4(SYS_wait4, v, &st, 0, 0);
    check("exec_child_killed", A_WIFSIGNALED(st), st);
    return finish();
}

int test_main(void) {
    /* The image the vfork child runs: no system calls at all. */
    if (a_argc() > 1 && str_eq(a_argv()[1], "spin"))
        for (;;) a_relax();
    return a_in_session(body);
}
