/* getcwd: follows renames of the working directory or its parents; a
 * removed working directory gives ENOENT; symbolic links are resolved to
 * the real path. */
#include "a_sys.h"

#define D "/tmp/a_getcwd_moves"

static char buf[256];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_a", a_mkdir(D "/a", 0700), 0);
    expect("setup_inner", a_mkdir(D "/a/inner", 0700), 0);
    expect("chdir_inner", syscall1(SYS_chdir, D "/a/inner"), 0);
    expect("rename_parent", syscall5(SYS_renameat2, AT_FDCWD, D "/a", AT_FDCWD, D "/b", 0), 0);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("follows_parent_rename", str_eq(buf, D "/b/inner"), buf[0]);
    expect("rename_self", syscall5(SYS_renameat2, AT_FDCWD, D "/b/inner", AT_FDCWD, D "/b/moved", 0), 0);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("follows_own_rename", str_eq(buf, D "/b/moved"), buf[0]);
    expect("setup_link", syscall3(SYS_symlinkat, "b/moved", AT_FDCWD, D "/l"), 0);
    expect("chdir_via_link", syscall1(SYS_chdir, D "/l"), 0);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("link_resolved", str_eq(buf, D "/b/moved"), buf[0]);
    expect("rmdir_cwd", a_rmdir(D "/b/moved"), 0);
    expect("removed_cwd_enoent", syscall2(SYS_getcwd, buf, sizeof buf), -ENOENT);
    expect("relative_open_in_removed", a_open("x", O_RDWR | O_CREAT, 0600), -ENOENT);
    expect("chdir_dotdot_from_removed", syscall1(SYS_chdir, ".."), 0);
    syscall2(SYS_getcwd, buf, sizeof buf);
    check("back_in_parent", str_eq(buf, D "/b"), buf[0]);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_b", a_rmdir(D "/b"), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
