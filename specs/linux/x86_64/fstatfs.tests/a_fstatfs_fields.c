/* fstatfs: the same answer as statfs for the same file, through file,
 * directory and O_PATH descriptors. */
#include "a_sys.h"

#define F "/tmp/a_fstatfs_fields"

static int same(struct a_statfs *a, struct a_statfs *b) {
    return a->f_type == b->f_type && a->f_bsize == b->f_bsize && a->f_blocks == b->f_blocks &&
           a->f_fsid[0] == b->f_fsid[0] && a->f_fsid[1] == b->f_fsid[1] && a->f_namelen == b->f_namelen &&
           a->f_frsize == b->f_frsize && a->f_flags == b->f_flags;
}

int test_main(void) {
    struct a_statfs p, f;
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("statfs_path", syscall2(SYS_statfs, F, &p), 0);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("fstatfs_file", syscall2(SYS_fstatfs, fd, &f), 0);
    check("file_same_as_statfs", same(&p, &f), f.f_type);
    a_close(fd);
    fd = a_open("/tmp", O_RDONLY | O_DIRECTORY, 0);
    expect("fstatfs_dir", syscall2(SYS_fstatfs, fd, &f), 0);
    check("dir_same_fs", same(&p, &f), f.f_type);
    a_close(fd);
    fd = a_open(F, O_PATH, 0);
    expect("fstatfs_path_fd", syscall2(SYS_fstatfs, fd, &f), 0);
    check("path_fd_same_fs", same(&p, &f), f.f_type);
    a_close(fd);
    fd = a_open(F, O_WRONLY, 0);
    expect("fstatfs_wronly", syscall2(SYS_fstatfs, fd, &f), 0);
    expect("namelen", f.f_namelen, 255);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
