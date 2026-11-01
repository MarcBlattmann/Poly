/* rmdir(2) permissions: write and search on the parent (EACCES otherwise);
 * in a sticky directory only the owner of the directory being removed,
 * the owner of the sticky directory, or root may remove it (EPERM); a
 * mount point is EBUSY; a read-only mount is EROFS.
 * poly-diff: needs=root (chowns directories to other users, changes uid,
 * mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_rmdir_perm"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 rmdir_(const char *p) { return syscall1(SYS_rmdir, p); }
static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

static int child_user(void) {
    expect("c_setgroups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    /* no write permission on the parent */
    expect("c_ro_parent", rmdir_(D "/ro/x"), -EACCES);
    expect("c_ro_parent_missing", rmdir_(D "/ro/nope"), -ENOENT);
    expect("c_ro_parent_full", rmdir_(D "/ro/full"), -EACCES);
    /* no search permission */
    expect("c_nox_parent", rmdir_(D "/nox/x"), -EACCES);
    /* sticky, world-writable directory owned by 2000 */
    expect("c_sticky_theirs", rmdir_(D "/sticky/theirs"), -EPERM);
    expect("c_sticky_mine", rmdir_(D "/sticky/mine"), 0);
    expect("c_sticky_missing", rmdir_(D "/sticky/nope"), -ENOENT);
    expect("c_sticky_theirs_full", rmdir_(D "/sticky/theirs_full"), -EPERM);
    /* sticky directory owned by the user: anything in it may go */
    expect("c_mysticky_theirs", rmdir_(D "/mysticky/theirs"), 0);
    /* a mount point and a read-only mount */
    expect("c_mountpoint", rmdir_(D "/mp/m"), -EBUSY);
    expect("c_rofs", rmdir_(D "/m/x"), -EROFS);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_ro_x", a_mkdir(D "/ro/x", 0777), 0);
    expect("setup_ro_x_own", chown_(D "/ro/x", 1000, 1000), 0);
    expect("setup_ro_full", a_mkdir(D "/ro/full", 0777), 0);
    expect("setup_ro_full_f", a_make_file(D "/ro/full/f", "x", 0644), 0);
    expect("setup_ro_own", chown_(D "/ro", 2000, 2000), 0);
    expect("setup_nox", a_mkdir(D "/nox", 0777), 0);
    expect("setup_nox_x", a_mkdir(D "/nox/x", 0777), 0);
    expect("setup_nox_mode", syscall2(SYS_chmod, D "/nox", 0776), 0);
    expect("setup_sticky", a_mkdir(D "/sticky", 0777), 0);
    expect("setup_sticky_mode", syscall2(SYS_chmod, D "/sticky", 01777), 0);
    expect("setup_sticky_own", chown_(D "/sticky", 2000, 2000), 0);
    expect("setup_sticky_theirs", a_mkdir(D "/sticky/theirs", 0777), 0);
    expect("setup_sticky_theirs_own", chown_(D "/sticky/theirs", 3000, 3000), 0);
    expect("setup_sticky_theirs_full", a_mkdir(D "/sticky/theirs_full", 0777), 0);
    expect("setup_sticky_theirs_full_f", a_make_file(D "/sticky/theirs_full/f", "x", 0644), 0);
    expect("setup_sticky_theirs_full_own", chown_(D "/sticky/theirs_full", 3000, 3000), 0);
    expect("setup_sticky_mine", a_mkdir(D "/sticky/mine", 0755), 0);
    expect("setup_sticky_mine_own", chown_(D "/sticky/mine", 1000, 1000), 0);
    expect("setup_mysticky", a_mkdir(D "/mysticky", 0755), 0);
    expect("setup_mysticky_mode", syscall2(SYS_chmod, D "/mysticky", 01755), 0);
    expect("setup_mysticky_own", chown_(D "/mysticky", 1000, 1000), 0);
    expect("setup_mysticky_theirs", a_mkdir(D "/mysticky/theirs", 0755), 0);
    expect("setup_mysticky_theirs_own", chown_(D "/mysticky/theirs", 3000, 3000), 0);
    expect("setup_mp", a_mkdir(D "/mp", 0777), 0);
    expect("setup_mp_mode", syscall2(SYS_chmod, D "/mp", 0777), 0);
    expect("setup_mp_m", a_mkdir(D "/mp/m", 0777), 0);
    expect("setup_mp_m_own", chown_(D "/mp/m", 1000, 1000), 0);
    expect("mount_mp", syscall5(SYS_mount, "none", D "/mp/m", "tmpfs", 0, "mode=0777"), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount_m", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0777"), 0);
    expect("setup_m_x", a_mkdir(D "/m/x", 0777), 0);
    expect("setup_m_x_own", chown_(D "/m/x", 1000, 1000), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", rmdir_(D "/m/x"), -EROFS);
    expect("root_rofs_missing", rmdir_(D "/m/nope"), -EROFS);
    expect("root_mountpoint", rmdir_(D "/mp/m"), -EBUSY);
    expect("root_mountpoint_root", rmdir_(D "/m"), -EBUSY);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    /* root passes the permission and sticky checks */
    expect("root_ro_x", rmdir_(D "/ro/x"), 0);
    expect("root_sticky_theirs", rmdir_(D "/sticky/theirs"), 0);

    expect("umount_m", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("umount_mp", syscall2(SYS_umount2, D "/mp/m", 0), 0);
    expect("root_mp_m_after_umount", rmdir_(D "/mp/m"), 0);
    expect("cleanup_mp", rmdir_(D "/mp"), 0);
    expect("cleanup_m", rmdir_(D "/m"), 0);
    expect("cleanup_mysticky", rmdir_(D "/mysticky"), 0);
    expect("cleanup_theirs_full_f", a_unlink(D "/sticky/theirs_full/f"), 0);
    expect("cleanup_theirs_full", rmdir_(D "/sticky/theirs_full"), 0);
    expect("cleanup_sticky", rmdir_(D "/sticky"), 0);
    expect("cleanup_nox_x", rmdir_(D "/nox/x"), 0);
    expect("cleanup_nox", rmdir_(D "/nox"), 0);
    expect("cleanup_ro_full_f", a_unlink(D "/ro/full/f"), 0);
    expect("cleanup_ro_full", rmdir_(D "/ro/full"), 0);
    expect("cleanup_ro", rmdir_(D "/ro"), 0);
    expect("cleanup_rmdir", rmdir_(D), 0);
    return finish();
}
