/* truncate of a program file that a process is running is ETXTBSY, also
 * to the size it already has; a negative length is still EINVAL. Once
 * the process has ended, the same file can be truncated. The program
 * copies itself to /tmp and runs the copy, which tries it on its own
 * file and reports the error number as its exit status.
 * poly-diff: syscalls=off (a child process runs another program)
 */
#include "a_sys.h"

#define EXE "/tmp/a_truncate_etxtbsy.exe"

static char copy[8192];

static int run_copy(char *mode) {
    char *argv[] = {EXE, mode, 0};
    char *envp[] = {0};
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall3(SYS_execve, EXE, argv, envp);
        syscall1(SYS_exit_group, 99);
    }
    i32 status = 0;
    if (syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return -1;
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 200 + A_WTERMSIG(status);
}

int test_main(void) {
    if (a_argc() > 1) {
        char m = a_argv()[1][0];
        struct a_stat st;
        a_stat_path(a_self_path(), &st);
        i64 len = m == 'z' ? 0 : m == 's' ? st.st_size : m == 'g' ? st.st_size + 1 : -1;
        return (int)-syscall2(SYS_truncate, a_self_path(), len);
    }
    a_block_sigchld();
    struct a_stat st;
    i64 src = a_open(a_self_path(), O_RDONLY, 0);
    i64 dst = a_open(EXE, O_WRONLY | O_CREAT | O_TRUNC, 0755);
    check("setup_open", src >= 0 && dst >= 0, dst);
    i64 n, total = 0;
    while ((n = a_read(src, copy, sizeof copy)) > 0) total += a_write(dst, copy, n);
    a_close(src);
    a_close(dst);
    check("setup_copied", total > 0, total);
    expect("running_to_zero", run_copy("z"), ETXTBSY);
    expect("running_same_size", run_copy("s"), ETXTBSY);
    expect("running_grow", run_copy("g"), ETXTBSY);
    expect("running_negative", run_copy("n"), EINVAL);
    a_stat_path(EXE, &st);
    expect("size_kept", st.st_size, total);
    expect("after_exit", syscall2(SYS_truncate, EXE, 0), 0);
    a_stat_path(EXE, &st);
    expect("after_exit_size", st.st_size, 0);
    expect("cleanup_unlink", a_unlink(EXE), 0);
    return finish();
}
