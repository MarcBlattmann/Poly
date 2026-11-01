/* utimes(2) permissions: explicit times need ownership (EPERM), NULL
 * needs ownership or write permission (EACCES); root may do both; a
 * read-only mount is EROFS.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_utimes_perm"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 utimes_(const char *p, const void *t) { return syscall2(SYS_utimes, p, t); }

static int child_user(void) {
    struct a_timeval t[2] = {{300, 3}, {400, 4}};
    expect("c_setgroups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    expect("c_theirs_rw_explicit", utimes_(D "/theirs_rw", t), -EPERM);
    expect("c_theirs_rw_now", utimes_(D "/theirs_rw", 0), 0);
    expect("c_theirs_ro_explicit", utimes_(D "/theirs_ro", t), -EPERM);
    expect("c_theirs_ro_now", utimes_(D "/theirs_ro", 0), -EACCES);
    expect("c_mine_mode0_explicit", utimes_(D "/mine0", t), 0);
    expect("c_mine_mode0_now", utimes_(D "/mine0", 0), 0);
    expect("c_rofs", utimes_(D "/m/mine", 0), -EROFS);
    /* EINVAL still comes first */
    struct a_timeval bad[2] = {{0, 2000000}, {0, 0}};
    expect("c_einval_before_eperm", utimes_(D "/theirs_ro", bad), -EINVAL);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_theirs_rw", a_make_file(D "/theirs_rw", "x", 0644), 0);
    expect("setup_theirs_rw_mode", syscall2(SYS_chmod, D "/theirs_rw", 0646), 0);
    expect("setup_theirs_rw_own", syscall3(SYS_chown, D "/theirs_rw", 2000, 2000), 0);
    expect("setup_theirs_ro", a_make_file(D "/theirs_ro", "x", 0644), 0);
    expect("setup_theirs_ro_own", syscall3(SYS_chown, D "/theirs_ro", 2000, 2000), 0);
    expect("setup_mine0", a_make_file(D "/mine0", "x", 0), 0);
    expect("setup_mine0_own", syscall3(SYS_chown, D "/mine0", 1000, 1000), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0755"), 0);
    expect("setup_m_mine", a_make_file(D "/m/mine", "x", 0666), 0);
    expect("setup_m_mine_own", syscall3(SYS_chown, D "/m/mine", 1000, 1000), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", utimes_(D "/m/mine", 0), -EROFS);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    struct a_timeval t[2] = {{5, 5}, {6, 6}};
    expect("root_explicit", utimes_(D "/theirs_ro", t), 0);
    expect("root_now", utimes_(D "/theirs_ro", 0), 0);

    expect("umount", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_mine0", a_unlink(D "/mine0"), 0);
    expect("cleanup_theirs_ro", a_unlink(D "/theirs_ro"), 0);
    expect("cleanup_theirs_rw", a_unlink(D "/theirs_rw"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
