/* readlink: a buffer whose start is writable but which runs into an
 * unmapped page: what happens when the text does or does not reach the
 * hole. The bytes before the hole are written, then EFAULT. */
#include "a_sys.h"

#define D "/tmp/a_readlink_partial_buf"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char *map = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *end;
    check("setup_map", (i64)map > 0, (i64)map > 0);
    expect("setup_hole", syscall2(SYS_munmap, map + 4096, 4096), 0);
    end = map + 4096;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_link", syscall2(SYS_symlink, "0123456789", D "/l"), 0);
    /* Text fits before the hole even though bufsiz reaches past it. */
    expect("fits_before_hole", rl(D "/l", end - 10, 100), 10);
    check("fits_before_hole_text", mem_eq(end - 10, "0123456789", 10), end[-10]);
    /* Truncated to what fits: bufsiz stops at the hole. */
    memset(end - 4, 'z', 4);
    expect("bufsiz_to_hole", rl(D "/l", end - 4, 4), 4);
    check("bufsiz_to_hole_text", mem_eq(end - 4, "0123", 4), end[-4]);
    /* Text crosses the hole. */
    memset(end - 4, 'z', 4);
    expect("crosses_hole", rl(D "/l", end - 4, 100), -EFAULT);
    check("crosses_hole_start_written", mem_eq(end - 4, "0123", 4), end[-4]);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    expect("cleanup_map", syscall2(SYS_munmap, map, 4096), 0);
    return finish();
}
