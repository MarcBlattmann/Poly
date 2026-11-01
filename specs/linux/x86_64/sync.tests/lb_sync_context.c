/* sync(2) works in any state of the process: with every descriptor
 * closed, in a child, with an unlinked open file, after a removed working
 * directory; it needs no privilege and always returns 0. */
#include "a_sys.h"

#define D "/tmp/lb_sync_context"
#define SYS_sync 162

static int child(void) {
    expect("child_sync", syscall0(SYS_sync), 0);
    for (int fd = 0; fd < 3; fd++) a_close(fd);
    i64 r = syscall0(SYS_sync);
    /* no output possible now: report through the exit status */
    return r == 0 ? 0 : 3;
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT, 0600);
    expect("write", a_write(fd, "x", 1), 1);
    expect("unlink_open", a_unlink(D "/f"), 0);
    expect("unlinked_open_file", syscall0(SYS_sync), 0);
    expect("still_readable", syscall4(SYS_pread64, fd, &fd, 0, 0), 0);
    a_close(fd);
    expect("setup_gone", a_mkdir(D "/gone", 0755), 0);
    expect("chdir_gone", syscall1(SYS_chdir, D "/gone"), 0);
    expect("rmdir_gone", a_rmdir(D "/gone"), 0);
    expect("removed_cwd", syscall0(SYS_sync), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_status", status, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
