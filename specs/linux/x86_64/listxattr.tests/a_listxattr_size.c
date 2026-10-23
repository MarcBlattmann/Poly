/* listxattr: size 0 returns the length the list needs without copying
 * (the buffer pointer is not used); a size smaller than the list, even by
 * one byte, is ERANGE (also with a NULL buffer) and writes nothing; equal
 * or larger sizes copy the list. Sizes above 65536 (65537, 2^32, SIZE_MAX)
 * are accepted and act as a 64 KiB buffer. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_listxattr 194
#define ERANGE 34
#define D "/tmp/a_listxattr_size"
#define F D "/f"
#define BAD ((void *)8)

static char buf[256];

static void fill(void) {
    for (int i = 0; i < 256; i++) buf[i] = '#';
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_a", syscall5(SYS_setxattr, F, "user.a", "1", 1, 0), 0);
    expect("setup_bb", syscall5(SYS_setxattr, F, "user.bb", "2", 1, 0), 0);
    expect("size0", syscall3(SYS_listxattr, F, buf, 0), 15);
    expect("size0_null", syscall3(SYS_listxattr, F, 0, 0), 15);
    expect("size0_bad_ptr", syscall3(SYS_listxattr, F, BAD, 0), 15);
    fill();
    expect("size1", syscall3(SYS_listxattr, F, buf, 1), -ERANGE);
    expect("size7", syscall3(SYS_listxattr, F, buf, 7), -ERANGE);
    expect("size14", syscall3(SYS_listxattr, F, buf, 14), -ERANGE);
    check("erange_untouched", buf[0] == '#' && buf[13] == '#', buf[0]);
    expect("size1_null", syscall3(SYS_listxattr, F, 0, 1), -ERANGE);
    expect("size14_bad_ptr", syscall3(SYS_listxattr, F, BAD, 14), -ERANGE);
    fill();
    expect("size15", syscall3(SYS_listxattr, F, buf, 15), 15);
    check("size15_nul_end", buf[14] == 0, buf[14]);
    check("size15_after", buf[15] == '#', buf[15]);
    fill();
    expect("size16", syscall3(SYS_listxattr, F, buf, 16), 15);
    check("size16_after", buf[15] == '#', buf[15]);
    expect("size65536", syscall3(SYS_listxattr, F, buf, 65536), 15);
    expect("size65537", syscall3(SYS_listxattr, F, buf, 65537), 15);
    expect("size_2_32", syscall3(SYS_listxattr, F, buf, 1L << 32), 15);
    expect("size_2_32_plus_1", syscall3(SYS_listxattr, F, buf, (1L << 32) + 1), 15);
    expect("size_max", syscall3(SYS_listxattr, F, buf, -1L), 15);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
