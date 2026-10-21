/* stat: the old stat call reports the same 144 bytes as fstat on an open
 * descriptor and newfstatat without flags, for files, directories,
 * devices and through symbolic links; lstat differs only for links. */
#include "a_sys.h"

#define D "/tmp/a_stat_agree"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

static int same(const char *path) {
    struct a_stat a, b, c;
    memset(&a, 0x5a, sizeof a);
    memset(&b, 0xa5, sizeof b);
    memset(&c, 0x33, sizeof c);
    if (st(path, &a) != 0) return -1;
    if (syscall4(SYS_newfstatat, AT_FDCWD, path, &b, 0) != 0) return -2;
    i64 fd = a_open(path, O_PATH, 0);
    if (fd < 0) return -3;
    i64 r = syscall2(SYS_fstat, fd, &c);
    a_close(fd);
    if (r != 0) return -4;
    if (!mem_eq(&a, &b, sizeof a)) return -5;
    if (!mem_eq(&a, &c, sizeof a)) return -6;
    return 1;
}

int test_main(void) {
    a_unlink(D "/f");
    a_unlink(D "/l");
    a_unlink(D "/ld");
    a_rmdir(D "/sub");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/f", "some bytes", 0640), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/ld"), 0);

    expect("file_agrees", same(D "/f"), 1);
    expect("dir_agrees", same(D), 1);
    expect("subdir_agrees", same(D "/sub"), 1);
    expect("link_agrees", same(D "/l"), 1);
    expect("dirlink_agrees", same(D "/ld"), 1);
    expect("devnull_agrees", same("/dev/null"), 1);
    expect("root_agrees", same("/"), 1);

    struct a_stat a, b;
    st(D "/f", &a);
    expect("lstat_file", syscall2(SYS_lstat, D "/f", &b), 0);
    check("lstat_file_same", mem_eq(&a, &b, sizeof a), 0);
    expect("lstat_link", syscall2(SYS_lstat, D "/l", &b), 0);
    expect("lstat_link_mode", b.st_mode, S_IFLNK | 0777);
    expect("lstat_link_size", b.st_size, 1);
    check("stat_link_is_target", st(D "/l", &b) == 0 && mem_eq(&a, &b, sizeof a), 0);

    /* The same file through an open descriptor after the name is gone. */
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("unlink_open_file", a_unlink(D "/f"), 0);
    expect("stat_unlinked_enoent", st(D "/f", &b), -ENOENT);
    expect("link_now_dangling", st(D "/l", &b), -ENOENT);
    syscall2(SYS_fstat, fd, &b);
    expect("fstat_nlink_0", b.st_nlink, 0);
    check("fstat_same_inode", b.st_ino == a.st_ino, 0);
    a_close(fd);

    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_ld", a_unlink(D "/ld"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
