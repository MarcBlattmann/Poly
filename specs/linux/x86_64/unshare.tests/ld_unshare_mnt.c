/* unshare(CLONE_NEWNS) gives the caller a copy of its mount table.
 * After making the copy private (mount(NULL, "/", NULL, MS_REC |
 * MS_PRIVATE, NULL)), a tmpfs mounted in it and files made there are
 * seen by the caller (and its children) only: the test's namespace still
 * sees the empty directory, and the mount goes away with the namespace.
 * The working directory and root stay where they were.
 * poly-diff: needs=root (a new mount namespace and mount need CAP_SYS_ADMIN)
 */
#include "a_sys.h"

#define SYS_mount 165
#define SYS_unshare 272
#define CLONE_NEWNS 0x20000 /* linux/sched.h */
/* linux/mount.h */
#define MS_REC 0x4000
#define MS_PRIVATE 0x40000

#define DIR "/tmp/ld_unshare_mnt"

int test_main(void) {
    a_block_sigchld();
    a_rmdir(DIR);
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        expect("unshare_newns", syscall1(SYS_unshare, CLONE_NEWNS), 0);
        expect("make_private", syscall5(SYS_mount, 0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
        expect("mount_tmpfs", syscall5(SYS_mount, "none", DIR, "tmpfs", 0, 0), 0);
        expect("make_file", a_make_file(DIR "/inner", "x", 0644), 0);
        struct a_stat st;
        expect("file_seen_inside", a_stat_path(DIR "/inner", &st), 0);
        struct a_statfs fs;
        syscall2(SYS_statfs, DIR, &fs);
        expect("tmpfs_magic_inside", fs.f_type, 0x01021994);
        char cwd[32];
        memset(cwd, 0, sizeof cwd);
        syscall2(SYS_getcwd, cwd, sizeof cwd);
        check("cwd_kept", str_eq(cwd, "/"), 0);
        a_shared()[0] = passed;
        a_shared()[1] = failed;
        syscall1(SYS_exit_group, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_exit_0", status, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    struct a_stat st;
    expect("file_not_seen_outside", a_stat_path(DIR "/inner", &st), -ENOENT);
    expect("rmdir_empty_dir", a_rmdir(DIR), 0);
    return finish();
}
