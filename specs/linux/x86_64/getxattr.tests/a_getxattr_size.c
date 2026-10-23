/* getxattr buffer size: size 0 returns the value's length and copies
 * nothing (the pointer is not used: NULL or an unmapped address work); a
 * size smaller than the value is ERANGE and leaves the buffer untouched;
 * a size equal to or larger than the value copies exactly the value and
 * nothing after it. Sizes above 65536 (65537, 2^32, 2^32 + 2, SIZE_MAX)
 * are accepted and act as a large buffer. A NULL buffer with a size large
 * enough is EFAULT, with a size too small ERANGE. A zero-length value
 * returns 0 whatever the buffer. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define D "/tmp/a_getxattr_size"
#define BAD ((void *)8)

static char buf[1100];
static char val[1000];

static i64 get(const char *name, void *b, i64 size) { return syscall4(SYS_getxattr, D "/f", name, b, size); }

static int untouched(int from, int to) {
    for (int i = from; i < to; i++)
        if (buf[i] != '#') return 0;
    return 1;
}

static void fill(void) {
    for (int i = 0; i < (int)sizeof buf; i++) buf[i] = '#';
}

int test_main(void) {
    for (int i = 0; i < (int)sizeof val; i++) val[i] = (char)(i * 7 + 1);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "hello", 5, 0), 0);
    expect("setup_set_big", syscall5(SYS_setxattr, D "/f", "user.big", val, sizeof val, 0), 0);
    expect("setup_set_empty", syscall5(SYS_setxattr, D "/f", "user.e", 0, 0, 0), 0);
    fill();
    expect("size0_length", get("user.a", buf, 0), 5);
    check("size0_copies_nothing", untouched(0, 16), 0);
    expect("size0_null", get("user.a", 0, 0), 5);
    expect("size0_bad_pointer", get("user.a", BAD, 0), 5);
    expect("size0_big_length", get("user.big", 0, 0), 1000);
    expect("size_short_erange", get("user.a", buf, 4), -ERANGE);
    expect("size_1_erange", get("user.a", buf, 1), -ERANGE);
    check("erange_untouched", untouched(0, 16), 0);
    expect("size_exact", get("user.a", buf, 5), 5);
    check("exact_value", mem_eq(buf, "hello", 5), 0);
    check("exact_tail_untouched", untouched(5, 16), 0);
    fill();
    expect("size_larger", get("user.a", buf, 16), 5);
    check("larger_value", mem_eq(buf, "hello", 5), 0);
    check("larger_tail_untouched", untouched(5, 16), 0);
    expect("size_65537", get("user.a", buf, 65537), 5);
    expect("size_2_32", get("user.a", buf, (i64)1 << 32), 5);
    expect("size_2_32_plus_2", get("user.a", buf, ((i64)1 << 32) + 2), 5);
    expect("size_max", get("user.a", buf, -1), 5);
    check("size_max_value", mem_eq(buf, "hello", 5), 0);
    expect("big_short_erange", get("user.big", buf, 999), -ERANGE);
    fill();
    expect("big_exact", get("user.big", buf, 1000), 1000);
    check("big_value", mem_eq(buf, val, sizeof val), 0);
    check("big_tail_untouched", untouched(1000, 1100), 0);
    expect("big_size_max", get("user.big", buf, -1), 1000);
    expect("null_short_erange", get("user.a", 0, 1), -ERANGE);
    expect("null_large_efault", get("user.a", 0, 100), -EFAULT);
    expect("bad_large_efault", get("user.a", BAD, 5), -EFAULT);
    expect("empty_value", get("user.e", buf, sizeof buf), 0);
    expect("empty_value_size0", get("user.e", 0, 0), 0);
    expect("empty_value_bad_pointer", get("user.e", BAD, 10), 0);
    expect("empty_value_null_size_max", get("user.e", 0, -1), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
