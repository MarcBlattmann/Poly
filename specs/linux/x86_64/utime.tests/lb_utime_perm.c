/* utime(2) by a user other than the owner: explicit times need ownership
 * (EPERM otherwise, even with write permission); NULL ("now") is allowed
 * with write permission, else EACCES; root may do both; a read-only mount
 * is EROFS.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_utime_perm"
#define SYS_utime 132
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

struct utimbuf {
    i64 actime, modtime;
};

static i64 utime_(const char *p, const void *t) { return syscall2(SYS_utime, p, t); }

static int child_user(void) {
    struct utimbuf t = {300, 400};
    struct a_stat st;
    expect("c_setgroups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    expect("c_theirs_writable_explicit", utime_(D "/theirs_rw", &t), -EPERM);
    expect("c_theirs_writable_now", utime_(D "/theirs_rw", 0), 0);
    expect("c_theirs_ro_explicit", utime_(D "/theirs_ro", &t), -EPERM);
    expect("c_theirs_ro_now", utime_(D "/theirs_ro", 0), -EACCES);
    expect("c_theirs_ro_unchanged", a_stat_path(D "/theirs_ro", &st) == 0 && st.st_mtime_sec == 50, 1);
    expect("c_mine_ro_explicit", utime_(D "/mine_ro", &t), 0);
    expect("c_mine_ro_now", utime_(D "/mine_ro", 0), 0);
    expect("c_rofs_explicit", utime_(D "/m/mine", &t), -EROFS);
    expect("c_rofs_now", utime_(D "/m/mine", 0), -EROFS);
    expect("c_rofs_theirs", utime_(D "/m/theirs", &t), -EROFS);
    return finish();
}

int test_main(void) {
    struct utimbuf old = {50, 50};
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_theirs_rw", a_make_file(D "/theirs_rw", "x", 0666), 0);
    expect("setup_theirs_rw_mode", syscall2(SYS_chmod, D "/theirs_rw", 0666), 0);
    expect("setup_theirs_rw_own", syscall3(SYS_chown, D "/theirs_rw", 2000, 2000), 0);
    expect("setup_theirs_ro", a_make_file(D "/theirs_ro", "x", 0644), 0);
    expect("setup_theirs_ro_own", syscall3(SYS_chown, D "/theirs_ro", 2000, 2000), 0);
    expect("setup_theirs_ro_times", utime_(D "/theirs_ro", &old), 0);
    expect("setup_mine_ro", a_make_file(D "/mine_ro", "x", 0444), 0);
    expect("setup_mine_ro_own", syscall3(SYS_chown, D "/mine_ro", 1000, 1000), 0);
    expect("setup_m", a_mkdir(D "/m", 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0755"), 0);
    expect("setup_m_mine", a_make_file(D "/m/mine", "x", 0666), 0);
    expect("setup_m_mine_own", syscall3(SYS_chown, D "/m/mine", 1000, 1000), 0);
    expect("setup_m_theirs", a_make_file(D "/m/theirs", "x", 0644), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_rofs", utime_(D "/m/mine", &old), -EROFS);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    struct utimbuf t = {600, 700};
    expect("root_theirs_explicit", utime_(D "/theirs_ro", &t), 0);
    expect("root_theirs_now", utime_(D "/theirs_ro", 0), 0);

    expect("umount", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_mine_ro", a_unlink(D "/mine_ro"), 0);
    expect("cleanup_theirs_ro", a_unlink(D "/theirs_ro"), 0);
    expect("cleanup_theirs_rw", a_unlink(D "/theirs_rw"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
