/* link(2) permissions: write and search on the new parent, search on the
 * old path (EACCES); with fs.protected_hardlinks=1 (the default) a caller
 * without CAP_FOWNER may only link files it owns or may both read and
 * write (and that are regular, not set-user-ID, not executable
 * set-group-ID), else EPERM; different mounts are EXDEV; a read-only mount
 * EROFS.
 * poly-diff: needs=root (chowns files to other users, changes uid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_link_perm"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_mknodat 259
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 link_(const char *a, const char *b) { return syscall2(SYS_link, a, b); }
static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

static int child_user(void) {
    expect("c_setgroups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    expect("c_own_file", link_(D "/mine", D "/w/mine2"), 0);
    expect("c_own_file_mode0", link_(D "/mine0", D "/w/mine0b"), 0);
    expect("c_theirs_rw", link_(D "/theirs_rw", D "/w/a"), 0);
    expect("c_theirs_ro", link_(D "/theirs_ro", D "/w/b"), -EPERM);
    expect("c_theirs_none", link_(D "/theirs_none", D "/w/c"), -EPERM);
    expect("c_theirs_suid_rw", link_(D "/theirs_suid", D "/w/d"), -EPERM);
    expect("c_theirs_sgid_x_rw", link_(D "/theirs_sgidx", D "/w/e"), -EPERM);
    expect("c_theirs_sgid_nox_rw", link_(D "/theirs_sgid", D "/w/f"), 0);
    expect("c_theirs_fifo_rw", link_(D "/theirs_fifo", D "/w/g"), -EPERM);
    expect("c_theirs_symlink", link_(D "/theirs_link", D "/w/h"), -EPERM);
    expect("c_ro_parent", link_(D "/mine", D "/ro/x"), -EACCES);
    expect("c_ro_parent_existing", link_(D "/mine", D "/ro/existing"), -EEXIST);
    expect("c_closed_old", link_(D "/closed/f", D "/w/i"), -EACCES);
    expect("c_exdev", link_(D "/mine", D "/mnt/x"), -EXDEV);
    expect("c_rofs", link_(D "/m/f", D "/m/g"), -EROFS);
    return finish();
}

int test_main(void) {
    a_block_sigchld();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_w", a_mkdir(D "/w", 0777), 0);
    expect("setup_w_mode", syscall2(SYS_chmod, D "/w", 0777), 0);
    expect("setup_mine", a_make_file(D "/mine", "x", 0644), 0);
    expect("setup_mine_own", chown_(D "/mine", 1000, 1000), 0);
    expect("setup_mine0", a_make_file(D "/mine0", "x", 0), 0);
    expect("setup_mine0_own", chown_(D "/mine0", 1000, 1000), 0);
    static const struct {
        const char *path;
        u32 mode;
    } theirs[] = {{D "/theirs_rw", 0666}, {D "/theirs_ro", 0644}, {D "/theirs_none", 0600},
                  {D "/theirs_suid", 04666}, {D "/theirs_sgidx", 02676}, {D "/theirs_sgid", 02666}};
    for (unsigned i = 0; i < sizeof theirs / sizeof theirs[0]; i++) {
        expect("setup_theirs", a_make_file(theirs[i].path, "x", 0600), 0);
        expect("setup_theirs_own", chown_(theirs[i].path, 2000, 2000), 0);
        expect("setup_theirs_mode", syscall2(SYS_chmod, theirs[i].path, theirs[i].mode), 0);
    }
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/theirs_fifo", S_IFIFO | 0666, 0), 0);
    expect("setup_fifo_own", chown_(D "/theirs_fifo", 2000, 2000), 0);
    expect("setup_fifo_mode", syscall2(SYS_chmod, D "/theirs_fifo", 0666), 0);
    expect("setup_link", syscall2(SYS_symlink, "theirs_none", D "/theirs_link"), 0);
    expect("setup_link_own", syscall3(94, D "/theirs_link", 2000, 2000), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_ro_existing", a_make_file(D "/ro/existing", "x", 0644), 0);
    expect("setup_closed", a_mkdir(D "/closed", 0700), 0);
    expect("setup_closed_f", a_make_file(D "/closed/f", "x", 0666), 0);
    expect("setup_closed_f_own", chown_(D "/closed/f", 1000, 1000), 0);
    expect("setup_mnt", a_mkdir(D "/mnt", 0777), 0);
    expect("mount_mnt", syscall5(SYS_mount, "none", D "/mnt", "tmpfs", 0, "mode=0777"), 0);
    expect("setup_m", a_mkdir(D "/m", 0777), 0);
    expect("mount_m", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0777"), 0);
    expect("setup_m_f", a_make_file(D "/m/f", "x", 0644), 0);
    expect("setup_m_f_own", chown_(D "/m/f", 1000, 1000), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("root_exdev", link_(D "/mine", D "/mnt/x"), -EXDEV);
    expect("root_exdev_existing", link_(D "/mine", D "/mnt"), -EEXIST);
    expect("root_rofs", link_(D "/m/f", D "/m/g"), -EROFS);
    expect("root_rofs_existing", link_(D "/m/f", D "/m/f"), -EEXIST);
    expect("root_theirs_none", link_(D "/theirs_none", D "/rootlink"), 0);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    expect("umount_m", syscall2(SYS_umount2, D "/m", 0), 0);
    expect("umount_mnt", syscall2(SYS_umount2, D "/mnt", 0), 0);
    static const char *rm[] = {D "/w/mine2", D "/w/mine0b", D "/w/a", D "/w/f", D "/rootlink",
                               D "/theirs_link", D "/theirs_fifo", D "/theirs_rw", D "/theirs_ro", D "/theirs_none",
                               D "/theirs_suid", D "/theirs_sgidx", D "/theirs_sgid", D "/mine", D "/mine0",
                               D "/ro/existing", D "/closed/f"};
    for (unsigned i = 0; i < sizeof rm / sizeof rm[0]; i++) expect("cleanup_unlink", a_unlink(rm[i]), 0);
    static const char *dirs[] = {D "/w", D "/ro", D "/closed", D "/mnt", D "/m", D};
    for (unsigned i = 0; i < sizeof dirs / sizeof dirs[0]; i++) expect("cleanup_rmdir", a_rmdir(dirs[i]), 0);
    return finish();
}
