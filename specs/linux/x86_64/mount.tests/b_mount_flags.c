/* mount flags: MS_RDONLY makes creating a file EROFS; MS_REMOUNT without
 * MS_RDONLY makes it writable again (files kept); MS_BIND mounts a
 * directory at a second place (same files, changes visible both ways);
 * a read-only bind remount (MS_REMOUNT|MS_BIND|MS_RDONLY) is read-only
 * there only; statfs f_flags shows ST_RDONLY (1). */
#include "poly_ipc.h"

#define D "/tmp/b_mount_flags"
#define B "/tmp/b_mount_flags_bind"

struct k_statfs {
    i64 f_type, f_bsize;
    u64 f_blocks, f_bfree, f_bavail, f_files, f_ffree;
    int f_fsid[2];
    i64 f_namelen, f_frsize, f_flags, f_spare[4];
};

int test_main(void) {
    for (int i = 0; i < 3; i++) {
        syscall2(SYS_umount2, B, MNT_DETACH);
        syscall2(SYS_umount2, D, MNT_DETACH);
    }
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, B, 0755);
    i64 r = syscall5(SYS_mount, "b_ro", D, "tmpfs", MS_RDONLY, 0);
    check("mount_rdonly", r == 0, r);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, D "/f", O_CREAT | O_WRONLY, 0644);
    check("create_on_rdonly_erofs", fd == -EROFS, fd);
    struct k_statfs sf;
    syscall2(SYS_statfs, D, &sf);
    check("statfs_rdonly_flag", (sf.f_flags & 1) != 0, sf.f_flags);
    r = syscall5(SYS_mount, 0, D, 0, MS_REMOUNT, 0);
    check("remount_rw", r == 0, r);
    fd = syscall4(SYS_openat, AT_FDCWD, D "/f", O_CREAT | O_WRONLY, 0644);
    check("create_after_remount", fd >= 0, fd);
    write((int)fd, "data", 4);
    close((int)fd);

    r = syscall5(SYS_mount, D, B, 0, MS_BIND, 0);
    check("bind_mount", r == 0, r);
    char buf[8];
    fd = syscall4(SYS_openat, AT_FDCWD, B "/f", O_RDONLY, 0);
    r = read((int)fd, buf, sizeof buf);
    check("bind_shows_same_file", r == 4 && mem_eq(buf, "data", 4), r);
    close((int)fd);
    close((int)syscall4(SYS_openat, AT_FDCWD, B "/g", O_CREAT | O_WRONLY, 0644));
    fd = syscall4(SYS_openat, AT_FDCWD, D "/g", O_RDONLY, 0);
    check("bind_changes_visible_both_ways", fd >= 0, fd);
    close((int)fd);
    r = syscall5(SYS_mount, 0, B, 0, MS_REMOUNT | MS_BIND | MS_RDONLY, 0);
    check("bind_remount_rdonly", r == 0, r);
    fd = syscall4(SYS_openat, AT_FDCWD, B "/h", O_CREAT | O_WRONLY, 0644);
    check("bind_readonly_erofs", fd == -EROFS, fd);
    fd = syscall4(SYS_openat, AT_FDCWD, D "/h", O_CREAT | O_WRONLY, 0644);
    check("original_still_writable", fd >= 0, fd);
    close((int)fd);

    check("umount_bind", syscall2(SYS_umount2, B, 0) == 0, 0);
    check("umount_tmpfs", syscall2(SYS_umount2, D, 0) == 0, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, B, AT_REMOVEDIR);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
