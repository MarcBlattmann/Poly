/* sync() changes no process state: umask, working directory, pid and the
 * lowest free descriptor are the same before and after. */
#include "a_sys.h"

#define SYS_sync 162

int test_main(void) {
    char before[256], after[256];
    i64 old = syscall1(SYS_umask, 027);
    expect("umask_start", old, 022);
    expect("chdir", syscall1(SYS_chdir, "/tmp"), 0);
    i64 n1 = syscall2(SYS_getcwd, before, sizeof before);
    i64 pid = syscall0(SYS_getpid);
    expect("sync", syscall0(SYS_sync), 0);
    expect("umask_kept", syscall1(SYS_umask, 022), 027);
    i64 n2 = syscall2(SYS_getcwd, after, sizeof after);
    check("cwd_kept", n1 == n2 && n1 > 0 && str_eq(before, after), n2);
    expect("pid_kept", syscall0(SYS_getpid), pid);
    i64 fd = a_open("/dev/null", O_RDONLY, 0);
    expect("lowest_fd", fd, 3);
    a_close(fd);
    return finish();
}
