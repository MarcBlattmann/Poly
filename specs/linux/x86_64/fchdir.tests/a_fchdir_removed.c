/* fchdir into a directory that was removed while open: allowed; getcwd
 * then fails with ENOENT and nothing can be created there. */
#include "a_sys.h"

#define D "/tmp/a_fchdir_removed"

int test_main(void) {
    char buf[128];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_gone", a_mkdir(D "/gone", 0700), 0);
    i64 d = a_open(D "/gone", O_RDONLY | O_DIRECTORY, 0);
    expect("rmdir_while_open", a_rmdir(D "/gone"), 0);
    expect("fchdir_into_removed", syscall1(SYS_fchdir, d), 0);
    expect("getcwd_enoent", syscall2(SYS_getcwd, buf, sizeof buf), -ENOENT);
    expect("create_enoent", a_open("x", O_RDWR | O_CREAT, 0600), -ENOENT);
    expect("mkdir_enoent", a_mkdir("x", 0700), -ENOENT);
    expect("absolute_paths_still_work", syscall1(SYS_chdir, D), 0);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("cwd_restored", str_eq(buf, D), buf[0]);
    a_close(d);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
