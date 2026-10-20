/* vfork then execve: the parent resumes when the child's execve
 * succeeds, while the new program runs as the same child process. Here
 * the child runs this program again with argument "child"; that image
 * makes no system call at all (it spins), so the parent ends it with
 * SIGKILL and wait4 reports the kill. Had the execve failed, the child
 * would have exited with status 127 instead. */
#include "a_sys.h"

int test_main(void) {
    if (a_argc() > 1 && str_eq(a_argv()[1], "child"))
        for (;;) a_relax();
    a_block_sigchld();
    char *argv[] = {"a_vfork_exec", "child", 0};
    char *envp[] = {0};
    const char *self = a_self_path();
    volatile i64 exec_result = 1;
    i64 pid = syscall0(SYS_vfork);
    if (pid == 0) {
        exec_result = syscall3(SYS_execve, self, argv, envp);
        syscall1(SYS_exit_group, 127);
    }
    i64 k = syscall2(SYS_kill, pid, SIGKILL);
    i32 status = 0;
    i64 w = syscall4(SYS_wait4, pid, &status, 0, 0);
    check("vfork", pid > 0, pid);
    expect("exec_did_not_return", exec_result, 1);
    expect("kill_exec_image", k, 0);
    expect("wait4", w, pid);
    check("exec_image_was_running", A_WIFSIGNALED(status) && A_WTERMSIG(status) == SIGKILL, status);
    return finish();
}
