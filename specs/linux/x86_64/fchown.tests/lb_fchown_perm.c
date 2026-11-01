/* fchown(2) by a user other than root: a descriptor (even one opened for
 * writing by root) grants nothing: only the owner may change the group, to
 * one of its groups; descriptors on a read-only mount are EROFS.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_fchown_perm"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 fchown_(i64 fd, i64 u, i64 g) { return syscall3(SYS_fchown, fd, u, g); }
static i64 theirs, mine, ro;

static int child_user(void) {
    u32 groups[1] = {3000};
    struct a_stat st;
    expect("c_setgroups", syscall2(SYS_setgroups, 1, groups), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    expect("c_theirs_take", fchown_(theirs, 1000, -1), -EPERM);
    expect("c_theirs_group", fchown_(theirs, -1, 1000), -EPERM);
    expect("c_theirs_noop", fchown_(theirs, -1, -1), 0);
    expect("c_mine_group", fchown_(mine, -1, 3000), 0);
    expect("c_mine_gid", syscall2(SYS_fstat, mine, &st) == 0 ? (i64)st.st_gid : -1, 3000);
    expect("c_mine_foreign", fchown_(mine, -1, 2000), -EPERM);
    expect("c_mine_give", fchown_(mine, 2000, -1), -EPERM);
    expect("c_rofs", fchown_(ro, -1, -1), -EROFS);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_theirs", a_make_file(D "/theirs", "x", 0666), 0);
    expect("setup_theirs_own", syscall3(SYS_chown, D "/theirs", 2000, 2000), 0);
    expect("setup_mine", a_make_file(D "/mine", "x", 0644), 0);
    expect("setup_mine_own", syscall3(SYS_chown, D "/mine", 1000, 1000), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0755"), 0);
    expect("setup_ro", a_make_file(D "/m/f", "x", 0644), 0);
    expect("setup_ro_own", syscall3(SYS_chown, D "/m/f", 1000, 1000), 0);
    theirs = a_open(D "/theirs", O_RDWR, 0);
    mine = a_open(D "/mine", O_RDONLY, 0);
    ro = a_open(D "/m/f", O_RDONLY, 0);
    check("opened", theirs >= 0 && mine >= 0 && ro >= 0, ro);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", fchown_(ro, 0, 0), -EROFS);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    expect("close_ro", a_close(ro), 0);
    expect("umount", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_mine", a_unlink(D "/mine"), 0);
    expect("cleanup_theirs", a_unlink(D "/theirs"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
