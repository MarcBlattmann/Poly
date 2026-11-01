/* fchmod(2) by a user other than root: a descriptor grants nothing, only
 * the owner may change the mode (EPERM); set-group-ID is dropped when the
 * caller is not in the file's group; a descriptor on a read-only mount is
 * EROFS, before ownership.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_fchmod_owner"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 fchmod_(i64 fd, i64 mode) { return syscall2(SYS_fchmod, fd, mode); }
static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }
static u32 fmode(i64 fd) {
    struct a_stat st;
    if (syscall2(SYS_fstat, fd, &st)) return 0xffffffff;
    return st.st_mode;
}

static i64 theirs, mine, mine2000, ro_mine;

static int child_user(void) {
    expect("c_setgroups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    /* opened read-write by root before: still not the owner */
    expect("c_theirs_eperm", fchmod_(theirs, 0600), -EPERM);
    expect("c_theirs_unchanged", fmode(theirs), S_IFREG | 0644);
    expect("c_mine_ok", fchmod_(mine, 02700), 0);
    expect("c_mine_sgid_kept", fmode(mine), S_IFREG | 02700);
    expect("c_mine2000_sgid", fchmod_(mine2000, 02750), 0);
    expect("c_mine2000_sgid_dropped", fmode(mine2000), S_IFREG | 0750);
    expect("c_rofs", fchmod_(ro_mine, 0600), -EROFS);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_theirs", a_make_file(D "/theirs", "x", 0666), 0);
    expect("setup_theirs_own", chown_(D "/theirs", 2000, 2000), 0);
    expect("setup_mine", a_make_file(D "/mine", "x", 0644), 0);
    expect("setup_mine_own", chown_(D "/mine", 1000, 1000), 0);
    expect("setup_mine2000", a_make_file(D "/mine2000", "x", 0644), 0);
    expect("setup_mine2000_own", chown_(D "/mine2000", 1000, 2000), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0755"), 0);
    expect("setup_ro_mine", a_make_file(D "/m/mine", "x", 0644), 0);
    expect("setup_ro_mine_own", chown_(D "/m/mine", 1000, 1000), 0);
    theirs = a_open(D "/theirs", O_RDWR, 0);
    mine = a_open(D "/mine", O_RDONLY, 0);
    mine2000 = a_open(D "/mine2000", O_RDONLY, 0);
    ro_mine = a_open(D "/m/mine", O_RDONLY, 0);
    check("opened", theirs >= 0 && mine >= 0 && mine2000 >= 0 && ro_mine >= 0, ro_mine);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", fchmod_(ro_mine, 0600), -EROFS);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    expect("root_theirs", fchmod_(theirs, 02604), 0);
    expect("root_theirs_mode", fmode(theirs), S_IFREG | 02604);

    expect("close_ro", a_close(ro_mine), 0);
    expect("umount", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_mine2000", a_unlink(D "/mine2000"), 0);
    expect("cleanup_mine", a_unlink(D "/mine"), 0);
    expect("cleanup_theirs", a_unlink(D "/theirs"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
