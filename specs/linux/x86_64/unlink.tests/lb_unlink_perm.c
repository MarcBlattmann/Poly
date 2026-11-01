/* unlink(2) permissions: write and search on the parent (EACCES; the
 * file's own mode and owner do not matter); in a sticky directory only the
 * file's owner, the directory's owner or root (EPERM otherwise); a file
 * with a mount on it is EBUSY; a read-only mount is EROFS.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_unlink_perm"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32
#define MS_BIND 4096

static i64 unlink_(const char *p) { return syscall1(SYS_unlink, p); }
static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

static int child_user(void) {
    expect("c_setgroups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    expect("c_ro_parent_mine", unlink_(D "/ro/mine"), -EACCES);
    expect("c_ro_parent_missing", unlink_(D "/ro/nope"), -ENOENT);
    expect("c_ro_parent_dir_eacces", unlink_(D "/ro/subdir"), -EACCES);
    expect("c_ro_parent_slash_enotdir", unlink_(D "/ro/mine/"), -ENOTDIR);
    expect("c_ro_parent_dot_eisdir", unlink_(D "/ro/subdir/."), -EISDIR);
    expect("c_nox_parent", unlink_(D "/nox/mine"), -EACCES);
    /* open (world-writable, not sticky) directory: anyone's file goes */
    expect("c_open_dir_theirs", unlink_(D "/open/theirs"), 0);
    expect("c_open_dir_theirs_mode0", unlink_(D "/open/theirs0"), 0);
    /* sticky directory owned by 2000 */
    expect("c_sticky_theirs", unlink_(D "/sticky/theirs"), -EPERM);
    expect("c_sticky_mine", unlink_(D "/sticky/mine"), 0);
    expect("c_sticky_missing", unlink_(D "/sticky/nope"), -ENOENT);
    /* sticky directory owned by the user */
    expect("c_mysticky_theirs", unlink_(D "/mysticky/theirs"), 0);
    /* a file with something mounted on it */
    expect("c_bind_target", unlink_(D "/open/bound"), -EBUSY);
    expect("c_rofs", unlink_(D "/m/mine"), -EROFS);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_ro_mine", a_make_file(D "/ro/mine", "x", 0644), 0);
    expect("setup_ro_mine_own", chown_(D "/ro/mine", 1000, 1000), 0);
    expect("setup_ro_subdir", a_mkdir(D "/ro/subdir", 0777), 0);
    expect("setup_nox", a_mkdir(D "/nox", 0777), 0);
    expect("setup_nox_mine", a_make_file(D "/nox/mine", "x", 0644), 0);
    expect("setup_nox_mode", syscall2(SYS_chmod, D "/nox", 0776), 0);
    expect("setup_open", a_mkdir(D "/open", 0777), 0);
    expect("setup_open_mode", syscall2(SYS_chmod, D "/open", 0777), 0);
    expect("setup_open_theirs", a_make_file(D "/open/theirs", "x", 0644), 0);
    expect("setup_open_theirs_own", chown_(D "/open/theirs", 3000, 3000), 0);
    expect("setup_open_theirs0", a_make_file(D "/open/theirs0", "x", 0), 0);
    expect("setup_sticky", a_mkdir(D "/sticky", 0777), 0);
    expect("setup_sticky_mode", syscall2(SYS_chmod, D "/sticky", 01777), 0);
    expect("setup_sticky_own", chown_(D "/sticky", 2000, 2000), 0);
    expect("setup_sticky_theirs", a_make_file(D "/sticky/theirs", "x", 0666), 0);
    expect("setup_sticky_theirs_own", chown_(D "/sticky/theirs", 3000, 3000), 0);
    expect("setup_sticky_mine", a_make_file(D "/sticky/mine", "x", 0644), 0);
    expect("setup_sticky_mine_own", chown_(D "/sticky/mine", 1000, 1000), 0);
    expect("setup_mysticky", a_mkdir(D "/mysticky", 0755), 0);
    expect("setup_mysticky_mode", syscall2(SYS_chmod, D "/mysticky", 01755), 0);
    expect("setup_mysticky_own", chown_(D "/mysticky", 1000, 1000), 0);
    expect("setup_mysticky_theirs", a_make_file(D "/mysticky/theirs", "x", 0644), 0);
    expect("setup_bound", a_make_file(D "/open/bound", "x", 0666), 0);
    expect("setup_bind_src", a_make_file(D "/src", "y", 0644), 0);
    expect("bind_mount", syscall5(SYS_mount, D "/src", D "/open/bound", 0, MS_BIND, 0), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount_m", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0777"), 0);
    expect("setup_m_mine", a_make_file(D "/m/mine", "x", 0644), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", unlink_(D "/m/mine"), -EROFS);
    expect("root_rofs_missing", unlink_(D "/m/nope"), -EROFS);
    expect("root_rofs_dot_eisdir", unlink_(D "/m/."), -EISDIR);
    expect("root_bind_target", unlink_(D "/open/bound"), -EBUSY);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    expect("root_sticky_theirs", unlink_(D "/sticky/theirs"), 0);
    expect("root_ro_mine", unlink_(D "/ro/mine"), 0);

    expect("umount_m", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("umount_bound", syscall2(SYS_umount2, D "/open/bound", 0), 0);
    expect("root_bound_after_umount", unlink_(D "/open/bound"), 0);
    expect("cleanup_src", unlink_(D "/src"), 0);
    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_mysticky", a_rmdir(D "/mysticky"), 0);
    expect("cleanup_sticky", a_rmdir(D "/sticky"), 0);
    expect("cleanup_open", a_rmdir(D "/open"), 0);
    expect("cleanup_nox_mine", unlink_(D "/nox/mine"), 0);
    expect("cleanup_nox", a_rmdir(D "/nox"), 0);
    expect("cleanup_ro_subdir", a_rmdir(D "/ro/subdir"), 0);
    expect("cleanup_ro", a_rmdir(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
