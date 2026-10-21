/* lstat: st_size follows writes and truncation; st_blocks counts 512-byte
 * units actually allocated (none for a hole); st_blksize is positive. */
#include "a_sys.h"

#define D "/tmp/a_lstat_size"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }

static char page[A_PAGE];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_EXCL, 0644);
    check("setup_open", fd >= 0, fd);
    expect("empty_size", (lstat(D "/f", &st), st.st_size), 0);
    expect("empty_blocks", st.st_blocks, 0);
    check("blksize_positive", st.st_blksize > 0, st.st_blksize);
    for (int i = 0; i < A_PAGE; i++) page[i] = 'p';
    expect("write_page", a_write(fd, page, A_PAGE), A_PAGE);
    expect("size_after_write", (lstat(D "/f", &st), st.st_size), A_PAGE);
    check("blocks_cover_page", st.st_blocks >= 8, st.st_blocks);
    expect("write_more", a_write(fd, page, 10), 10);
    expect("size_grows", (lstat(D "/f", &st), st.st_size), A_PAGE + 10);
    expect("ftruncate_small", syscall2(SYS_ftruncate, fd, 3), 0);
    expect("size_truncated", (lstat(D "/f", &st), st.st_size), 3);
    a_close(fd);
    expect("truncate_big", syscall2(SYS_truncate, D "/f", 1 << 20), 0);
    expect("size_big", (lstat(D "/f", &st), st.st_size), 1 << 20);
    check("hole_not_allocated", st.st_blocks < 64, st.st_blocks);

    expect("setup_sparse", a_make_file(D "/s", "", 0644), 0);
    expect("truncate_sparse", syscall2(SYS_truncate, D "/s", 10 << 20), 0);
    expect("sparse_size", (lstat(D "/s", &st), st.st_size), 10 << 20);
    expect("sparse_blocks_0", st.st_blocks, 0);

    expect("setup_link", syscall3(SYS_symlinkat, "s", AT_FDCWD, D "/l"), 0);
    expect("link_size_not_target", (lstat(D "/l", &st), st.st_size), 1);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_s", a_unlink(D "/s"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
