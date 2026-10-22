/* mknod: S_IFIFO makes a named pipe; dev is ignored; the pipe works like
 * one: a nonblocking writer without a reader gets ENXIO, data written is
 * read back, the size stays 0. */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_fifo"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

int test_main(void) {
    struct a_stat st;
    char buf[16];
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    expect("mkfifo", mk(D "/p", S_IFIFO | 0666, 0x0103), 0);
    expect("stat", a_lstat_path(D "/p", &st), 0);
    expect("mode", st.st_mode, S_IFIFO | 0644);
    expect("rdev_ignored", (i64)st.st_rdev, 0);
    expect("size", st.st_size, 0);
    expect("nlink", st.st_nlink, 1);
    expect("uid", st.st_uid, syscall0(SYS_geteuid));
    expect("gid", st.st_gid, syscall0(SYS_getegid));

    expect("writer_no_reader_enxio", a_open(D "/p", O_WRONLY | O_NONBLOCK, 0), -ENXIO);
    i64 r = a_open(D "/p", O_RDONLY | O_NONBLOCK, 0);
    check("reader_open", r >= 0, r);
    expect("read_empty_eof", a_read(r, buf, sizeof buf), 0);
    i64 w = a_open(D "/p", O_WRONLY | O_NONBLOCK, 0);
    check("writer_open", w >= 0, w);
    expect("read_empty_eagain", a_read(r, buf, sizeof buf), -EAGAIN);
    expect("write", a_write(w, "through", 7), 7);
    expect("size_still_0", a_stat_path(D "/p", &st) == 0 ? st.st_size : -1, 0);
    expect("read", a_read(r, buf, sizeof buf), 7);
    check("read_data", mem_eq(buf, "through", 7), 0);
    a_close(w);
    expect("read_after_writer_closed", a_read(r, buf, sizeof buf), 0);
    a_close(r);

    i64 fd = a_open(D "/p", O_RDWR | O_NONBLOCK, 0);
    check("open_rdwr", fd >= 0, fd);
    struct a_stat fst;
    expect("fstat", syscall2(SYS_fstat, fd, &fst), 0);
    expect("fstat_same_inode", fst.st_ino == st.st_ino, 1);
    expect("fstat_fifo", fst.st_mode & S_IFMT, S_IFIFO);
    a_close(fd);

    expect("again_eexist", mk(D "/p", S_IFIFO | 0644, 0), -EEXIST);
    expect("fifo_over_file_eexist", a_make_file(D "/f", "x", 0644) == 0 ? mk(D "/f", S_IFIFO | 0644, 0) : -1, -EEXIST);
    expect("file_unchanged", a_lstat_path(D "/f", &st) == 0 ? st.st_mode : 0, S_IFREG | 0644);

    expect("cleanup_p", a_unlink(D "/p"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
