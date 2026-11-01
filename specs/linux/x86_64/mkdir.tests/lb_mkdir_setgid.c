/* mkdir(2) in a set-group-ID directory: the new directory takes the
 * parent's group and is set-group-ID itself (even if mode did not ask for
 * it); a file created there takes the group but not the bit. A read-only
 * mount answers EROFS for a new name and EEXIST for an existing one.
 * poly-diff: needs=root (chowns a directory to another group, changes
 * gid, mounts)
 */
#include "a_sys.h"

#define D "/tmp/lb_mkdir_setgid"
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mount 165
#define SYS_umount2 166
#define MS_RDONLY 1
#define MS_REMOUNT 32

static i64 mkdir_(const char *p, i64 mode) { return syscall2(SYS_mkdir, p, mode); }

static int child_user(void) {
    struct a_stat st;
    expect("c_setgroups", syscall2(SYS_setgroups, 0, 0), 0);
    expect("c_setresgid", syscall3(SYS_setresgid, 1000, 1000, 1000), 0);
    expect("c_setresuid", syscall3(SYS_setresuid, 1000, 1000, 1000), 0);
    /* the user is not in group 2000, the directory still inherits it */
    expect("c_mkdir", mkdir_(D "/sg/u", 0755), 0);
    expect("c_stat", a_stat_path(D "/sg/u", &st), 0);
    expect("c_uid", st.st_uid, 1000);
    expect("c_gid", st.st_gid, 2000);
    expect("c_mode", st.st_mode, S_IFDIR | 02755);
    /* outside, the user's own gid */
    expect("c_mkdir_plain", mkdir_(D "/plain/u", 0755), 0);
    expect("c_stat_plain", a_stat_path(D "/plain/u", &st), 0);
    expect("c_gid_plain", st.st_gid, 1000);
    expect("c_mode_plain", st.st_mode, S_IFDIR | 0755);
    return finish();
}

int test_main(void) {
    struct a_stat st;
    a_block_sigchld();
    expect("setup_mkdir", mkdir_(D, 0755), 0);
    expect("setup_sg", mkdir_(D "/sg", 0777), 0);
    expect("setup_sg_chown", syscall3(SYS_chown, D "/sg", 0, 2000), 0);
    expect("setup_sg_chmod", syscall2(SYS_chmod, D "/sg", 02777), 0);
    expect("setup_plain", mkdir_(D "/plain", 0777), 0);
    expect("setup_plain_chmod", syscall2(SYS_chmod, D "/plain", 0777), 0);

    expect("root_mkdir", mkdir_(D "/sg/r", 0700), 0);
    expect("root_stat", a_stat_path(D "/sg/r", &st), 0);
    expect("root_gid_inherited", st.st_gid, 2000);
    expect("root_mode_sgid", st.st_mode, S_IFDIR | 02700);
    expect("root_file", a_make_file(D "/sg/file", "x", 0644), 0);
    expect("root_file_stat", a_stat_path(D "/sg/file", &st), 0);
    expect("file_gid_inherited", st.st_gid, 2000);
    expect("file_no_sgid", st.st_mode, S_IFREG | 0644);
    /* grandchild: inherited again */
    expect("grandchild", mkdir_(D "/sg/r/gc", 0750), 0);
    expect("grandchild_stat", a_stat_path(D "/sg/r/gc", &st), 0);
    expect("grandchild_gid", st.st_gid, 2000);
    expect("grandchild_mode", st.st_mode, S_IFDIR | 02750);

    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(child_user());
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_passed", status, 0);

    /* read-only mount */
    expect("setup_m", mkdir_(D "/m", 0755), 0);
    expect("mount", syscall5(SYS_mount, "none", D "/m", "tmpfs", 0, "mode=0755"), 0);
    expect("setup_m_existing", mkdir_(D "/m/existing", 0755), 0);
    expect("remount_ro", syscall5(SYS_mount, "none", D "/m", "tmpfs", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("rofs_new", mkdir_(D "/m/new", 0755), -EROFS);
    expect("rofs_existing_eexist", mkdir_(D "/m/existing", 0755), -EEXIST);
    expect("rofs_missing_parent", mkdir_(D "/m/nope/x", 0755), -ENOENT);
    expect("rofs_mountpoint_eexist", mkdir_(D "/m", 0755), -EEXIST);
    expect("umount", syscall2(SYS_umount2, D "/m", 0), 0);

    expect("cleanup_m", a_rmdir(D "/m"), 0);
    expect("cleanup_plain_u", a_rmdir(D "/plain/u"), 0);
    expect("cleanup_plain", a_rmdir(D "/plain"), 0);
    expect("cleanup_u", a_rmdir(D "/sg/u"), 0);
    expect("cleanup_gc", a_rmdir(D "/sg/r/gc"), 0);
    expect("cleanup_r", a_rmdir(D "/sg/r"), 0);
    expect("cleanup_file", a_unlink(D "/sg/file"), 0);
    expect("cleanup_sg", a_rmdir(D "/sg"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
