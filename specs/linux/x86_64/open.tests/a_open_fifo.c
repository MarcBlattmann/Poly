/* open(2) on a FIFO with O_NONBLOCK: reading ends open at once, a writing
 * end without a reader is ENXIO, with a reader it opens. */
#include "a_sys.h"

#define SYS_open 2
#define SYS_mknodat 259
#define D "/tmp/a_open_fifo"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

int test_main(void) {
    char buf[8];
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/p", S_IFIFO | 0600, 0), 0);

    expect("wronly_nonblock_no_reader_enxio", op(D "/p", O_WRONLY | O_NONBLOCK, 0), -ENXIO);
    i64 r = op(D "/p", O_RDONLY | O_NONBLOCK, 0);
    expect("rdonly_nonblock_opens", r, 3);
    expect("rdonly_nonblock_read_eof", a_read(r, buf, 8), 0);
    i64 w = op(D "/p", O_WRONLY | O_NONBLOCK, 0);
    expect("wronly_nonblock_with_reader", w, 4);
    expect("write_to_fifo", a_write(w, "pq", 2), 2);
    expect("read_from_fifo", a_read(r, buf, 8), 2);
    expect("read_from_fifo_data", mem_eq(buf, "pq", 2), 1);
    expect("empty_fifo_eagain", a_read(r, buf, 8), -EAGAIN);
    a_close(w);
    a_close(r);

    /* O_RDWR on a FIFO opens without waiting, even without O_NONBLOCK. */
    i64 rw = op(D "/p", O_RDWR, 0);
    expect("rdwr_opens", rw, 3);
    w = op(D "/p", O_WRONLY | O_NONBLOCK, 0);
    expect("rdwr_counts_as_reader", w, 4);
    a_close(w);
    a_close(rw);

    /* O_TRUNC and O_CREAT on an existing FIFO change nothing. */
    r = op(D "/p", O_RDONLY | O_NONBLOCK | O_TRUNC | O_CREAT, 0600);
    expect("fifo_trunc_creat_opens", r, 3);
    struct a_stat st;
    expect("fifo_still_fifo", syscall2(SYS_fstat, r, &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFIFO);
    a_close(r);

    expect("fifo_o_directory_enotdir", op(D "/p", O_RDONLY | O_NONBLOCK | O_DIRECTORY, 0), -ENOTDIR);

    expect("cleanup_unlink", a_unlink(D "/p"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
