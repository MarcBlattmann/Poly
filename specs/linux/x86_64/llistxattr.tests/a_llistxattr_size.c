/* llistxattr size handling on a file with names: size 0 returns the
 * needed length and writes nothing; any nonzero size smaller than the list
 * (down to one byte short) is ERANGE and writes nothing; an exact or
 * larger size copies the list to the start of the buffer and leaves the
 * rest untouched. Sizes above 65536 (65537, 2^32, 2^32 + 1, SIZE_MAX) act
 * as a 65536-byte buffer. A directory behaves the same. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_llistxattr 195
#define D "/tmp/a_llistxattr_size"
#define F D "/f"

static char buf[128];

static i64 ll(const char *p, i64 n) { return syscall3(SYS_llistxattr, p, buf, n); }

static void fill(void) {
    for (int i = 0; i < (int)sizeof buf; i++) buf[i] = '#';
}

static int untouched(int from, int to) {
    for (int i = from; i < to; i++)
        if (buf[i] != '#') return 0;
    return 1;
}

static int nuls(int n) {
    int c = 0;
    for (int i = 0; i < n; i++) c += buf[i] == 0;
    return c;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_set_a", syscall5(SYS_setxattr, F, "user.alpha", "1", 1, 0), 0);
    expect("setup_set_b", syscall5(SYS_setxattr, F, "user.b", "2", 1, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D, "user.dir", "3", 1, 0), 0);
    fill();
    expect("size0", ll(F, 0), 18);
    check("size0_untouched", untouched(0, sizeof buf), 0);
    expect("size1", ll(F, 1), -ERANGE);
    expect("size17", ll(F, 17), -ERANGE);
    check("erange_untouched", untouched(0, sizeof buf), 0);
    expect("size18", ll(F, 18), 18);
    check("size18_tail_untouched", untouched(18, sizeof buf), 0);
    check("size18_nuls", nuls(18) == 2 && buf[17] == 0, 0);
    fill();
    expect("size_larger", ll(F, 100), 18);
    check("larger_tail_untouched", untouched(18, sizeof buf), 0);
    check("larger_has_alpha",
          mem_eq(buf, "user.alpha\0", 11) || mem_eq(buf + 7, "user.alpha\0", 11), 0);
    check("larger_has_b", mem_eq(buf, "user.b\0", 7) || mem_eq(buf + 11, "user.b\0", 7), 0);
    expect("size_65536", ll(F, 65536), 18);
    expect("size_65537", ll(F, 65537), 18);
    expect("size_2_32", ll(F, 1L << 32), 18);
    expect("size_2_32_plus_1", ll(F, (1L << 32) + 1), 18);
    expect("size_max", ll(F, -1L), 18);
    expect("dir_size0", ll(D, 0), 9);
    expect("dir_size8", ll(D, 8), -ERANGE);
    expect("dir_size9", ll(D, 9), 9);
    check("dir_bytes", mem_eq(buf, "user.dir\0", 9), 0);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
