/* stat: how many symbolic links one lookup follows. A chain of 40 links
 * resolves; 41 is ELOOP. Links met in the middle of a path count too. */
#include "a_sys.h"

#define D "/tmp/a_stat_link_limit"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

/* D "/lN" for N in 0..99 */
static char *name(char *buf, int n) {
    char *p = str_cpy(buf, D "/l");
    if (n >= 10) *p++ = (char)('0' + n / 10);
    *p++ = (char)('0' + n % 10);
    *p = 0;
    return buf;
}

static void unlink_m(void) {
    char m[64] = D "/m0";
    a_unlink(m);
    for (int i = 1; i < 20; i++) {
        m[sizeof(D) + 1] = (char)('0' + i / 10);
        m[sizeof(D) + 2] = (char)('0' + i % 10);
        m[sizeof(D) + 3] = 0;
        a_unlink(m);
    }
}

int test_main(void) {
    struct a_stat s;
    char a[64], b[64];
    for (int i = 0; i <= 45; i++) a_unlink(name(a, i));
    unlink_m();
    a_unlink(D "/f");
    a_rmdir(D "/d");
    a_rmdir(D);

    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    /* l0 -> f, lN -> l(N-1) (relative targets): following lN takes N+1 links. */
    int ok = syscall3(SYS_symlinkat, "f", AT_FDCWD, name(a, 0)) == 0;
    for (int i = 1; i <= 45; i++) {
        name(b, i - 1);
        ok &= syscall3(SYS_symlinkat, b + sizeof(D), AT_FDCWD, name(a, i)) == 0;
    }
    check("setup_chain", ok, 0);
    expect("one_link", st(name(a, 0), &s), 0);
    expect("chain_of_39", st(name(a, 38), &s), 0);
    expect("chain_of_40", st(name(a, 39), &s), 0);
    expect("chain_of_40_is_file", s.st_mode & S_IFMT, S_IFREG);
    expect("chain_of_41_eloop", st(name(a, 40), &s), -ELOOP);
    expect("chain_of_46_eloop", st(name(a, 45), &s), -ELOOP);
    /* The limit is per lookup, across the path: l39 leads to the file, so
     * l39/x is ENOTDIR, but a chain to a directory can be continued. */
    expect("chain_40_then_component_enotdir", st(D "/l39/x", &s), -ENOTDIR);
    /* m0 -> d, mN -> m(N-1): m19 is a chain of 20 links to a directory. */
    ok = syscall3(SYS_symlinkat, "d", AT_FDCWD, D "/m0") == 0;
    char m[64] = D "/m0", prev[8] = "m0";
    for (int i = 1; i < 20; i++) {
        m[sizeof(D) + 1] = (char)('0' + i / 10);
        m[sizeof(D) + 2] = (char)('0' + i % 10);
        m[sizeof(D) + 3] = 0;
        ok &= syscall3(SYS_symlinkat, prev, AT_FDCWD, m) == 0;
        str_cpy(prev, m + sizeof(D));
    }
    check("setup_dir_chain", ok, 0);
    expect("dir_chain_20", st(D "/m19", &s), 0);
    expect("dir_chain_is_dir", s.st_mode & S_IFMT, S_IFDIR);
    expect("total_40_across_path", st(D "/m19/../l19", &s), 0);
    expect("total_41_across_path_eloop", st(D "/m19/../l20", &s), -ELOOP);
    expect("total_21_then_dotdot_ok", st(D "/m19/../l0", &s), 0);

    for (int i = 0; i <= 45; i++) a_unlink(name(a, i));
    unlink_m();
    a_unlink(D "/f");
    a_rmdir(D "/d");
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
