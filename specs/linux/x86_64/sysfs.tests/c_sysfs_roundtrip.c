/* Every index below the option-3 count names a type (option 2 returns
 * 0), the names are non-empty printable text without spaces, all
 * different, and option 1 maps each back to its own index. The count
 * itself and anything above it are EINVAL. Reading the same index twice
 * gives the same name.
 * poly-diff: syscalls=off (the number of calls depends on how many file system types the kernel has)
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139
#define MAXN 256

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

static char names[MAXN][64];

int test_main(void) {
    i64 n = sysfs3(3, 0, 0);
    check("count_in_range", n >= 1 && n <= MAXN, n);
    if (n < 1 || n > MAXN) return finish();
    int all_ok = 1, printable = 1, back = 1, again = 1;
    i64 bad = 0;
    for (i64 i = 0; i < n; i++) {
        char buf[4096];
        memset(buf, 0, sizeof buf);
        i64 r = sysfs3(2, i, (i64)buf);
        if (r != 0) all_ok = 0, bad = i;
        size_t len = str_len(buf);
        if (len == 0 || len >= 64) printable = 0, bad = i;
        for (size_t k = 0; k < len; k++)
            if (buf[k] <= ' ' || buf[k] > '~') printable = 0, bad = i;
        if (len < 64) str_cpy(names[i], buf);
        r = sysfs3(1, (i64)buf, 0);
        if (r != i) back = 0, bad = i;
        char buf2[4096];
        memset(buf2, 0, sizeof buf2);
        r = sysfs3(2, i, (i64)buf2);
        if (r != 0 || !str_eq(buf, buf2)) again = 0, bad = i;
    }
    check("every_index_ok", all_ok, bad);
    check("names_printable", printable, bad);
    check("name_maps_back", back, bad);
    check("same_name_twice", again, bad);
    int unique = 1;
    for (i64 i = 0; i < n; i++)
        for (i64 j = i + 1; j < n; j++)
            if (str_eq(names[i], names[j])) unique = 0, bad = j;
    check("names_unique", unique, bad);

    char buf[4096];
    i64 r = sysfs3(2, n, (i64)buf);
    check("index_count_einval", r == -EINVAL, r);
    r = sysfs3(2, n + 1, (i64)buf);
    check("index_count_plus1_einval", r == -EINVAL, r);
    r = sysfs3(2, n + 1000, (i64)buf);
    check("index_far_einval", r == -EINVAL, r);
    return finish();
}
