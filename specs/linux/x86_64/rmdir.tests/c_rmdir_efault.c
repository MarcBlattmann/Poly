/* rmdir with a path pointer it cannot read: NULL, a small address such as
 * 1, an unmapped page, or a string that runs into an unmapped page before
 * its NUL: EFAULT, and nothing is removed. A string whose NUL is the last
 * byte of a mapped page works. */
#include "a_sys.h"

#define D "/tmp/c_rmdir_efault"

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_x", a_mkdir(D "/x", 0755), 0);

    char *map = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("mmap", (i64)map > 0, 1);
    expect("munmap_second", syscall2(SYS_munmap, map + A_PAGE, A_PAGE), 0);

    expect("null_efault", rm(0), -EFAULT);
    expect("one_efault", rm((const char *)1), -EFAULT);
    expect("unmapped_efault", rm(map + A_PAGE), -EFAULT);
    expect("high_efault", rm((const char *)0xffff800000000000UL), -EFAULT);

    /* D "/x" without its NUL at the very end of the mapped page. */
    const char *p = D "/x";
    size_t n = str_len(p);
    char *s = map + A_PAGE - n;
    memcpy(s, p, n);
    expect("unterminated_efault", rm(s), -EFAULT);
    expect("x_kept", a_stat_path(D "/x", &st), 0);

    /* With the NUL as the last mapped byte. */
    s = map + A_PAGE - n - 1;
    memcpy(s, p, n + 1);
    expect("nul_at_page_end_works", rm(s), 0);
    expect("x_gone", a_lstat_path(D "/x", &st), -ENOENT);

    syscall2(SYS_munmap, map, A_PAGE);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
