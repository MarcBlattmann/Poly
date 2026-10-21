/* A named pipe behaves like an anonymous one: mknodat(S_IFIFO) makes it;
 * opening the read end with O_NONBLOCK succeeds with no writer, opening
 * the write end with O_NONBLOCK and no reader fails with ENXIO; with both
 * open, data flows; O_RDWR opens without waiting; lseek is ESPIPE. */
#include "poly_ipc.h"

#define SYS_mknodat 259
#define SYS_unlinkat 263
#define S_IFIFO 0010000
#define F "/tmp/b_pipe_fifo"

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    i64 r = syscall4(SYS_mknodat, AT_FDCWD, F, S_IFIFO | 0600, 0);
    check("mknod_fifo", r == 0, r);
    i64 w = syscall4(SYS_openat, AT_FDCWD, F, O_WRONLY | O_NONBLOCK, 0);
    check("write_open_no_reader_enxio", w == -ENXIO, w);
    i64 rd = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY | O_NONBLOCK, 0);
    check("read_open_nonblock_ok", rd >= 0, rd);
    char buf[8];
    r = read((int)rd, buf, sizeof buf);
    check("read_no_writer_ever_zero", r == 0, r);
    w = syscall4(SYS_openat, AT_FDCWD, F, O_WRONLY | O_NONBLOCK, 0);
    check("write_open_with_reader_ok", w >= 0, w);
    r = read((int)rd, buf, sizeof buf);
    check("empty_with_writer_eagain", r == -EAGAIN, r);
    r = write((int)w, "fifo", 4);
    check("write_fifo", r == 4, r);
    r = read((int)rd, buf, sizeof buf);
    check("read_fifo", r == 4 && mem_eq(buf, "fifo", 4), r);
    check("fifo_capacity", syscall2(SYS_fcntl, rd, F_GETPIPE_SZ) == 65536, 0);
    r = syscall3(SYS_lseek, rd, 0, 0);
    check("lseek_espipe", r == -ESPIPE, r);
    close((int)w);
    r = read((int)rd, buf, sizeof buf);
    check("eof_after_writer_closes", r == 0, r);
    close((int)rd);

    i64 rw = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR, 0);
    check("rdwr_open_does_not_block", rw >= 0, rw);
    write((int)rw, "z", 1);
    r = read((int)rw, buf, 1);
    check("rdwr_both_ends", r == 1 && buf[0] == 'z', r);
    close((int)rw);
    r = syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    check("cleanup", r == 0, r);
    return finish();
}
