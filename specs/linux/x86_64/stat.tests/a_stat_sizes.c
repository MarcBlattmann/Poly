/* stat: st_size and st_blocks of regular files: sparse files report the
 * length, not the allocation; written data allocates 512-byte blocks;
 * sizes beyond 4 GiB are reported in full. */
#include "a_sys.h"

#define D "/tmp/a_stat_sizes"
#define F D "/f"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

int test_main(void) {
    struct a_stat s;
    a_unlink(F);
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);

    expect("truncate_1m", syscall2(SYS_truncate, F, 1 << 20), 0);
    st(F, &s);
    expect("sparse_size", s.st_size, 1 << 20);
    check("sparse_few_blocks", s.st_blocks < (1 << 20) / 512, s.st_blocks);

    static char page[A_PAGE];
    memset(page, 'x', sizeof page);
    i64 fd = a_open(F, O_WRONLY, 0);
    expect("pwrite_page", syscall4(SYS_pwrite64, fd, page, sizeof page, 0), A_PAGE);
    syscall1(SYS_fsync, fd);
    st(F, &s);
    expect("size_unchanged_by_inner_write", s.st_size, 1 << 20);
    check("written_page_allocated", s.st_blocks >= A_PAGE / 512, s.st_blocks);

    expect("pwrite_past_end", syscall4(SYS_pwrite64, fd, page, 100, (1 << 20) + 10), 100);
    st(F, &s);
    expect("size_after_write_past_end", s.st_size, (1 << 20) + 110);

    i64 big = 5L << 30;
    expect("truncate_5g", syscall2(SYS_ftruncate, fd, big), 0);
    st(F, &s);
    expect("size_5g", s.st_size, big);
    check("size_5g_sparse", s.st_blocks < big / 512, s.st_blocks);
    expect("truncate_0", syscall2(SYS_ftruncate, fd, 0), 0);
    st(F, &s);
    expect("size_0", s.st_size, 0);
    expect("blocks_0", s.st_blocks, 0);
    a_close(fd);

    expect("cleanup_unlink", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
