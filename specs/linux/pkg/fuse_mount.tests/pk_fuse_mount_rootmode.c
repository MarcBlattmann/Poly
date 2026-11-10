/* mount(2) of "fuse": the type bits of rootmode= against the mount point,
 * and options that belong to another type. rootmode is octal; its file
 * type bits say what the root of the file system is, and that must fit the
 * mount point: a root that is not a directory (regular 100000, character
 * device 20000, block device 60000, fifo 10000, socket 140000; permission
 * bits make no difference) over a directory is ENOTDIR, and a directory
 * root (40000) over a regular file is ENOTDIR as well; a regular root over
 * a regular file mounts (0). Type bits that are no file type (0, 170000,
 * 30000) and a symbolic link are EINVAL, as is a value that is not octal
 * (8, 4x). blksize= is an option of the type fuseblk: with "fuse" or
 * "fuse.NAME" it is EINVAL whatever its value. user_id= and group_id= must
 * be decimal numbers (EINVAL otherwise). Every try uses a new descriptor.
 * (mount(8) fuse options, linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off (the descriptor numbers and option strings vary)
 */
#include "p_fuse.h"

#define D "/tmp/pk_fuse_mroot"
#define F "/tmp/pk_fuse_mroot_file"

static i64 try_on(const char *target, const char *fstype, const char *rest) {
    i64 fd = a_open("/dev/fuse", O_RDWR, 0);
    if (fd < 0) return fd;
    char opts[160];
    char *p = str_cpy(opts, "fd=");
    a_utoa(p, (u64)fd);
    p += str_len(p);
    str_cpy(p, rest);
    i64 r = syscall5(SYS_mount, "pk", target, fstype, MS_NOSUID | MS_NODEV, opts);
    if (r == 0) syscall2(SYS_umount2, target, MNT_DETACH);
    a_close(fd);
    return r;
}

#define IDS ",user_id=0,group_id=0"
#define BASE ",rootmode=40000" IDS

int test_main(void) {
    expect("setup", fz_setup(D), 0);
    a_unlink(F);
    expect("make_file", a_make_file(F, "x", 0644), 0);

    expect("dir_root_over_dir", try_on(D, "fuse", ",rootmode=40000" IDS), 0);
    expect("dir_root_leading_zero", try_on(D, "fuse", ",rootmode=040700" IDS), 0);
    expect("chardev_root_over_dir", try_on(D, "fuse", ",rootmode=20000" IDS), -ENOTDIR);
    expect("blockdev_root_over_dir", try_on(D, "fuse", ",rootmode=60000" IDS), -ENOTDIR);
    expect("fifo_root_over_dir", try_on(D, "fuse", ",rootmode=10000" IDS), -ENOTDIR);
    expect("socket_root_over_dir", try_on(D, "fuse", ",rootmode=140000" IDS), -ENOTDIR);
    expect("regular_0644_root_over_dir", try_on(D, "fuse", ",rootmode=100644" IDS), -ENOTDIR);
    expect("dir_root_over_file", try_on(F, "fuse", ",rootmode=40000" IDS), -ENOTDIR);
    expect("regular_root_over_file", try_on(F, "fuse", ",rootmode=100000" IDS), 0);
    expect("fifo_root_over_file", try_on(F, "fuse", ",rootmode=10000" IDS), 0);

    expect("type_bits_zero", try_on(D, "fuse", ",rootmode=0" IDS), -EINVAL);
    expect("type_bits_all", try_on(D, "fuse", ",rootmode=170000" IDS), -EINVAL);
    expect("type_bits_30000", try_on(D, "fuse", ",rootmode=30000" IDS), -EINVAL);
    expect("symlink_root_over_file", try_on(F, "fuse", ",rootmode=120777" IDS), -EINVAL);
    expect("digit_8", try_on(D, "fuse", ",rootmode=8" IDS), -EINVAL);
    expect("octal_then_letter", try_on(D, "fuse", ",rootmode=4x" IDS), -EINVAL);
    expect("rootmode_empty", try_on(D, "fuse", ",rootmode=" IDS), -EINVAL);

    expect("blksize_512", try_on(D, "fuse", BASE ",blksize=512"), -EINVAL);
    expect("blksize_subtype", try_on(D, "fuse.pk", BASE ",blksize=1024"), -EINVAL);
    expect("blksize_not_number", try_on(D, "fuse", BASE ",blksize=big"), -EINVAL);
    expect("same_without_blksize", try_on(D, "fuse.pk", BASE), 0);

    expect("user_id_text", try_on(D, "fuse", ",rootmode=40000,user_id=root,group_id=0"), -EINVAL);
    expect("group_id_empty", try_on(D, "fuse", ",rootmode=40000,user_id=0,group_id="), -EINVAL);
    expect("user_id_negative", try_on(D, "fuse", ",rootmode=40000,user_id=-2,group_id=0"), -EINVAL);

    a_unlink(F);
    fz_cleanup();
    return finish();
}
