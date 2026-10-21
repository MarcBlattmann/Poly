/* mount a tmpfs: on an existing directory the new file system hides what
 * was there (the directory's old contents come back after umount2), files
 * made in it are gone after umount2, statfs reports TMPFS_MAGIC; the
 * source name is free text for tmpfs; mounting twice on the same point
 * stacks (the second must be unmounted first). Errors: a missing mount
 * point ENOENT, a file system type that does not exist ENODEV, a mount
 * point that is a regular file ENOTDIR, a bad pointer EFAULT. */
#include "poly_ipc.h"

#define D "/tmp/b_mount_tmpfs"

struct k_statfs {
    i64 f_type, f_bsize;
    u64 f_blocks, f_bfree, f_bavail, f_files, f_ffree;
    int f_fsid[2];
    i64 f_namelen, f_frsize, f_flags, f_spare[4];
};

static int exists(const char *p) {
    i64 fd = syscall4(SYS_openat, AT_FDCWD, p, O_RDONLY, 0);
    if (fd < 0) return 0;
    close((int)fd);
    return 1;
}

int test_main(void) {
    syscall2(SYS_umount2, D, MNT_DETACH);
    syscall2(SYS_umount2, D, MNT_DETACH);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/under", 0);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    close((int)syscall4(SYS_openat, AT_FDCWD, D "/under", O_CREAT | O_WRONLY, 0644));

    i64 r = syscall5(SYS_mount, "b_src", D, "tmpfs", 0, "size=1m");
    check("mount_tmpfs_ok", r == 0, r);
    struct k_statfs sf;
    syscall2(SYS_statfs, D, &sf);
    check("statfs_tmpfs_magic", sf.f_type == TMPFS_MAGIC, sf.f_type);
    check("old_contents_hidden", !exists(D "/under"), 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, D "/inside", O_CREAT | O_WRONLY, 0644);
    check("file_in_tmpfs", fd >= 0, fd);
    close((int)fd);

    r = syscall5(SYS_mount, "b_src2", D, "tmpfs", 0, 0);
    check("stacked_mount_ok", r == 0, r);
    check("stacked_hides_first", !exists(D "/inside"), 0);
    r = syscall2(SYS_umount2, D, 0);
    check("umount_top", r == 0, r);
    check("first_visible_again", exists(D "/inside"), 0);
    r = syscall2(SYS_umount2, D, 0);
    check("umount_first", r == 0, r);
    check("tmpfs_files_gone", !exists(D "/inside"), 0);
    check("old_contents_back", exists(D "/under"), 0);

    r = syscall5(SYS_mount, "x", "/tmp/b_mount_missing", "tmpfs", 0, 0);
    check("missing_point_enoent", r == -ENOENT, r);
    r = syscall5(SYS_mount, "x", D, "b_nofs", 0, 0);
    check("unknown_type_enodev", r == -ENODEV, r);
    r = syscall5(SYS_mount, "x", D "/under", "tmpfs", 0, 0);
    check("file_point_enotdir", r == -ENOTDIR, r);
    r = syscall5(SYS_mount, "x", 8, "tmpfs", 0, 0);
    check("bad_target_efault", r == -EFAULT, r);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/under", 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
