/* openat2's size argument: the size of the caller's struct open_how.
 * Below 24 (OPEN_HOW_SIZE_VER0) EINVAL; above 4096 (a page) E2BIG; from
 * 24 to 4096 the first 24 bytes are the struct and every byte after them
 * must be zero (else E2BIG). The size is checked before the struct is
 * read (a NULL struct with size 0 is EINVAL, with 5000 E2BIG, with 24
 * EFAULT), and all of these before the path and dirfd.
 */
#include "a_sys.h"

#define SYS_openat2 437

/* linux/openat2.h */
struct open_how {
    u64 flags, mode, resolve;
};
static struct {
    struct open_how how;
    unsigned char tail[8192];
} big;

static i64 o2s(i64 d, const void *p, const void *how, u64 size) { return syscall4(SYS_openat2, d, p, how, size); }

static void good(const char *name, i64 fd) {
    expect(name, fd, 3);
    if (fd >= 0) a_close(fd);
}

int test_main(void) {
    big.how.flags = O_RDONLY | O_DIRECTORY;
    expect("size_0_einval", o2s(AT_FDCWD, "/", &big, 0), -EINVAL);
    expect("size_8_einval", o2s(AT_FDCWD, "/", &big, 8), -EINVAL);
    expect("size_23_einval", o2s(AT_FDCWD, "/", &big, 23), -EINVAL);
    good("size_24_ok", o2s(AT_FDCWD, "/", &big, 24));
    good("size_25_zero_tail_ok", o2s(AT_FDCWD, "/", &big, 25));
    good("size_32_zero_tail_ok", o2s(AT_FDCWD, "/", &big, 32));
    good("size_4096_zero_tail_ok", o2s(AT_FDCWD, "/", &big, 4096));
    expect("size_4097_e2big", o2s(AT_FDCWD, "/", &big, 4097), -E2BIG);
    expect("size_huge_e2big", o2s(AT_FDCWD, "/", &big, 1UL << 40), -E2BIG);
    expect("size_negative_e2big", o2s(AT_FDCWD, "/", &big, (u64)-1), -E2BIG);

    big.tail[4] = 1; /* byte 28 */
    expect("nonzero_byte_28_size_32_e2big", o2s(AT_FDCWD, "/", &big, 32), -E2BIG);
    good("nonzero_byte_28_outside_size_28", o2s(AT_FDCWD, "/", &big, 28));
    expect("nonzero_byte_28_size_29_e2big", o2s(AT_FDCWD, "/", &big, 29), -E2BIG);
    big.tail[4] = 0;
    big.tail[4071] = 0x80; /* byte 4095 */
    expect("nonzero_byte_4095_e2big", o2s(AT_FDCWD, "/", &big, 4096), -E2BIG);
    good("nonzero_byte_4095_size_4095", o2s(AT_FDCWD, "/", &big, 4095));
    big.tail[4071] = 0;

    expect("null_how_size_24_efault", o2s(AT_FDCWD, "/", 0, 24), -EFAULT);
    expect("null_how_size_0_einval", o2s(AT_FDCWD, "/", 0, 0), -EINVAL);
    expect("null_how_size_5000_e2big", o2s(AT_FDCWD, "/", 0, 5000), -E2BIG);
    expect("bad_size_before_path", o2s(AT_FDCWD, 0, &big, 8), -EINVAL);
    expect("bad_size_before_dirfd", o2s(99, "x", &big, 8), -EINVAL);
    expect("bad_tail_before_path", (big.tail[0] = 1, o2s(AT_FDCWD, 0, &big, 32)), -E2BIG);
    big.tail[0] = 0;
    expect("bad_how_before_dirfd", o2s(99, "x", 0, 24), -EFAULT);
    expect("bad_path_efault", o2s(AT_FDCWD, 0, &big, 24), -EFAULT);
    return finish();
}
