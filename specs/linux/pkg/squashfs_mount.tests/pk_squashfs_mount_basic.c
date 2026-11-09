/* mount("/dev/loopN", dir, "squashfs", MS_RDONLY, NULL) on a loop device that holds a squashfs
 * image mounts it read-only; so does a mount without MS_RDONLY, and a remount without
 * MS_RDONLY succeeds but the file system stays read-only (opening for writing is EROFS).
 * statfs: f_type 0x73717368, f_bsize = the image's block size (4096), f_blocks = bytes used
 * rounded up to blocks (1), f_bfree = f_bavail = 0, f_files = the number of inodes (6),
 * f_ffree 0, f_namelen 256, ST_RDONLY (1) in f_flags. umount2 of a mount with an open file
 * is EBUSY, MNT_DETACH works and the open file stays readable until it is closed; umount of
 * what is not a mount point is EINVAL. mount onto a regular file is ENOTDIR, onto a missing
 * directory ENOENT, an unknown file system type ENODEV, a squashfs mount of the same
 * device on the same directory again is allowed only after the first is gone.
 * Image: small-gzip (mksquashfs small -b 4096 -comp gzip -all-root, see the sheet).
 * poly-diff: needs=root (loop device, mount)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_sm_basic"

static const unsigned char img[] = {
    104,115,113,115,6,0,0,0,0,0,0,0,0,16,0,0,1,0,0,0,1,0,12,0,
    192,0,1,0,4,0,0,0,173,0,0,0,0,0,0,0,237,1,0,0,0,0,0,0,
    229,1,0,0,0,0,0,0,255,255,255,255,255,255,255,255,0,1,0,0,0,0,0,0,
    94,1,0,0,0,0,0,0,183,1,0,0,0,0,0,0,215,1,0,0,0,0,0,0,
    120,218,237,200,193,17,192,16,0,0,176,191,105,40,165,198,161,116,255,17,236,209,75,158,
    137,233,202,229,174,237,233,99,190,107,127,33,10,33,132,16,66,8,33,132,16,226,111,113,
    0,154,176,41,223,120,218,237,200,71,17,192,32,0,0,176,63,106,104,161,3,57,76,255,
    18,240,193,37,207,132,120,221,41,63,239,247,151,218,250,152,43,8,33,132,16,66,8,33,
    132,16,226,180,216,16,225,41,131,120,218,75,227,50,48,52,50,54,49,53,51,183,176,76,
    76,74,78,73,77,27,21,24,21,24,21,64,23,0,0,42,156,209,170,105,110,110,101,114,
    10,104,101,108,108,111,32,115,113,117,97,115,104,102,115,10,92,0,120,218,99,98,88,194,
    200,0,5,76,12,168,128,13,136,25,25,222,194,229,25,145,212,41,64,229,153,144,244,51,
    163,233,229,7,139,253,135,203,179,64,205,224,4,226,140,212,156,156,124,189,146,138,18,100,
    3,88,129,56,1,136,255,3,1,136,175,161,204,192,96,10,164,77,128,88,9,205,49,108,
    72,150,186,50,200,50,176,3,105,0,164,48,14,121,69,0,120,218,99,96,128,0,38,56,
    205,193,144,153,151,151,90,164,87,82,81,194,12,21,100,4,98,5,40,157,226,0,84,3,
    82,149,145,154,147,147,15,82,149,192,192,204,0,82,153,211,200,192,2,148,225,98,40,72,
    44,41,73,45,202,3,201,1,0,224,50,14,29,16,128,235,0,0,0,0,0,0,0,21,
    0,0,1,0,0,0,0,165,1,0,0,0,0,0,0,22,0,120,218,83,96,64,5,14,
    80,58,1,74,55,66,233,181,80,26,0,36,168,1,239,191,1,0,0,0,0,0,0,4,
    128,0,0,0,0,223,1,0,0,0,0,0,0,
};

static struct p_sq sq;
static char pb[160];

int test_main(void) {
    check("setup", p_sq_setup(&sq, D, img, sizeof img, 0, 0) == 0, 0);
    expect("mount_ro", p_sq_mount(&sq), 0);

    struct a_statfs fs;
    memset(&fs, 0xaa, sizeof fs);
    expect("statfs", syscall2(SYS_statfs, sq.mnt, &fs), 0);
    expect("f_type_squashfs", fs.f_type, 0x73717368);
    expect("f_bsize_4096", fs.f_bsize, 4096);
    expect("f_blocks_1", fs.f_blocks, 1);
    expect("f_bfree_0", fs.f_bfree, 0);
    expect("f_bavail_0", fs.f_bavail, 0);
    expect("f_files_6", fs.f_files, 6);
    expect("f_ffree_0", fs.f_ffree, 0);
    expect("f_namelen_256", fs.f_namelen, 256);
    expect("f_flags_rdonly", fs.f_flags & 1, 1);

    struct a_stat st;
    expect("stat_root", syscall2(SYS_stat, sq.mnt, &st), 0);
    expect("root_is_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("root_mode_755", st.st_mode & 07777, 0755);
    expect("root_nlink_3", st.st_nlink, 3);
    i64 fd = a_open(p_sq_path(&sq, pb, "/hello.txt"), O_RDONLY, 0);
    check("open_hello", fd >= 0, fd);
    char buf[32];
    memset(buf, 0, sizeof buf);
    expect("read_hello", a_read(fd, buf, sizeof buf), 15);
    check("hello_content", mem_eq(buf, "hello squashfs\n", 15), 0);

    /* remount: succeeds, stays read-only */
    expect("remount_rw_ok", syscall5(SYS_mount, 0, sq.mnt, 0, MS_REMOUNT, 0), 0);
    expect("still_erofs_after_remount", a_open(p_sq_path(&sq, pb, "/hello.txt"), O_WRONLY, 0), -EROFS);
    expect("remount_ro_ok", syscall5(SYS_mount, 0, sq.mnt, 0, MS_REMOUNT | MS_RDONLY, 0), 0);
    expect("statfs_after_remount", syscall2(SYS_statfs, sq.mnt, &fs), 0);
    expect("flags_still_rdonly", fs.f_flags & 1, 1);

    /* busy */
    expect("umount_busy", p_umount(sq.mnt), -EBUSY);
    expect("umount_detach", syscall2(SYS_umount2, sq.mnt, MNT_DETACH), 0);
    memset(buf, 0, sizeof buf);
    expect("open_file_still_reads", syscall4(SYS_pread64, fd, buf, 5, 6), 5);
    check("still_content", mem_eq(buf, "squas", 5), 0);
    a_close(fd);
    expect("path_gone_after_detach", syscall2(SYS_stat, p_sq_path(&sq, pb, "/hello.txt"), &st), -ENOENT);
    expect("umount_not_a_mount_einval", p_umount(sq.mnt), -EINVAL);

    /* mount without MS_RDONLY */
    expect("mount_without_ro_flag", syscall5(SYS_mount, sq.node, sq.mnt, "squashfs", 0, 0), 0);
    expect("statfs_2", syscall2(SYS_statfs, sq.mnt, &fs), 0);
    expect("rdonly_anyway", fs.f_flags & 1, 1);
    expect("write_erofs", a_open(p_sq_path(&sq, pb, "/new"), O_WRONLY | O_CREAT, 0644), -EROFS);
    expect("mount_again_same_dir_ebusy", p_sq_mount(&sq), -EBUSY);
    expect("umount_2", p_umount(sq.mnt), 0);

    /* bad targets and types */
    expect("mount_onto_file_enotdir", p_mount_squash(sq.node, sq.imgp, 0), -ENOTDIR);
    char missing[96];
    str_cat2(missing, sq.dir, "/nodir");
    expect("mount_onto_missing_enoent", p_mount_squash(sq.node, missing, 0), -ENOENT);
    expect("unknown_fstype_enodev", syscall5(SYS_mount, sq.node, sq.mnt, "nosuchfs", MS_RDONLY, 0), -ENODEV);
    expect("null_fstype_einval", syscall5(SYS_mount, sq.node, sq.mnt, 0, MS_RDONLY, 0), -EINVAL);
    expect("mount_after_all", p_sq_mount(&sq), 0);
    expect("final_umount", p_umount(sq.mnt), 0);

    p_sq_teardown(&sq);
    return finish();
}
