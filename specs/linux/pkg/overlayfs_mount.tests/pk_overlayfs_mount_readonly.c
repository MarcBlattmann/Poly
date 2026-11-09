/* An overlay mount without upperdir= is read-only and needs at least two
 * lowerdir entries (one entry alone is EINVAL). Its merged view is the
 * lowers, leftmost first. Every change fails with EROFS and statfs reports
 * ST_RDONLY. With an upperdir, MS_RDONLY (and a remount with MS_RDONLY)
 * makes the mount read-only, and a remount without it makes it writable
 * again. A single lowerdir is fine when an upperdir is given.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovm_ro"

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(B "/l1");
    pm_mkdirs(B "/l2/sub");
    pm_mkdirs(B "/l3");
    pm_mkdirs(B "/u");
    pm_mkdirs(B "/w");
    pm_mkdirs(B "/m");
    pm_put(B, "/l1/f", "first", 0644);
    pm_put(B, "/l2/f", "second", 0644);
    pm_put(B, "/l2/g", "gee", 0644);
    pm_put(B, "/l3/h", "aitch", 0644);

    /* a single lowerdir and nothing else */
    expect("single_lower_einval", pm_mount("ov", B "/m", "overlay", 0, "lowerdir=" B "/l1"), -EINVAL);
    /* two lowers, no upper */
    expect("two_lowers_mount", pm_mount("ov", B "/m", "overlay", 0, "lowerdir=" B "/l1:" B "/l2"), 0);
    pm_expect_list("readdir", B "/m", "f,g,sub");
    pm_expect_file("leftmost", B "/m/f", "first");
    pm_expect_file("second", B "/m/g", "gee");
    struct a_statfs fs;
    expect("statfs", syscall2(SYS_statfs, B "/m", &fs), 0);
    expect("statfs_type", fs.f_type, OVERLAYFS_SUPER_MAGIC);
    expect("statfs_rdonly", fs.f_flags & ST_RDONLY, ST_RDONLY);
    expect("create_erofs", a_make_file(B "/m/new", "x", 0644), -EROFS);
    expect("open_wronly_erofs", a_open(B "/m/f", O_WRONLY, 0), -EROFS);
    expect("open_rdwr_erofs", a_open(B "/m/f", O_RDWR, 0), -EROFS);
    expect("unlink_erofs", a_unlink(B "/m/f"), -EROFS);
    expect("mkdir_erofs", a_mkdir(B "/m/nd", 0755), -EROFS);
    expect("rmdir_erofs", a_rmdir(B "/m/sub"), -EROFS);
    expect("chmod_erofs", syscall3(SYS_chmod, B "/m/f", 0600, 0), -EROFS);
    expect("truncate_erofs", syscall2(SYS_truncate, B "/m/f", 0), -EROFS);
    expect("rename_erofs", syscall2(SYS_rename, B "/m/f", B "/m/f2"), -EROFS);
    expect("symlink_erofs", syscall3(SYS_symlink, "f", B "/m/s", 0), -EROFS);
    pm_expect_file("still_readable", B "/m/f", "first");
    expect("umount", pm_umount(B "/m", 0), 0);

    /* three lowers, order and merge */
    expect("three_lowers", pm_mount("ov", B "/m", "overlay", 0, "lowerdir=" B "/l3:" B "/l2:" B "/l1"), 0);
    pm_expect_list("readdir3", B "/m", "f,g,h,sub");
    pm_expect_file("f_from_second", B "/m/f", "second");
    expect("umount3", pm_umount(B "/m", 0), 0);

    /* one lowerdir is fine with an upperdir */
    expect("single_lower_with_upper",
           pm_mount("ov", B "/m", "overlay", 0, "lowerdir=" B "/l1,upperdir=" B "/u,workdir=" B "/w"), 0);
    expect("rw_create", a_make_file(B "/m/new", "n", 0644), 0);
    pm_expect_file("upper_has_new", B "/u/new", "n");
    expect("umount_single", pm_umount(B "/m", 0), 0);

    /* MS_RDONLY with an upperdir */
    const char *opts = "lowerdir=" B "/l1:" B "/l2,upperdir=" B "/u,workdir=" B "/w";
    expect("mount_ro_flag", pm_mount("ov", B "/m", "overlay", MS_RDONLY, opts), 0);
    expect("ro_flag_statfs", syscall2(SYS_statfs, B "/m", &fs), 0);
    expect("ro_flag_rdonly", fs.f_flags & ST_RDONLY, ST_RDONLY);
    expect("ro_flag_create_erofs", a_make_file(B "/m/more", "x", 0644), -EROFS);
    expect("ro_flag_write_lower_erofs", a_open(B "/m/g", O_WRONLY, 0), -EROFS);
    pm_expect_list("ro_flag_upper_unchanged", B "/u", "new");
    expect("remount_rw", pm_mount("ov", B "/m", "overlay", MS_REMOUNT, 0), 0);
    expect("rw_after_remount", a_make_file(B "/m/more", "x", 0644), 0);
    expect("remount_ro", pm_mount("ov", B "/m", "overlay", MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("ro_after_remount", a_make_file(B "/m/more2", "x", 0644), -EROFS);
    expect("statfs_after_remount", syscall2(SYS_statfs, B "/m", &fs), 0);
    expect("flag_after_remount", fs.f_flags & ST_RDONLY, ST_RDONLY);
    pm_expect_file("reads_still_work", B "/m/more", "x");
    expect("umount_final", pm_umount(B "/m", 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
