/* stat: names and inodes. Hard links share one inode and count in
 * st_nlink; rename keeps the inode; a name's file changes when another
 * file is renamed over it. Names are bytes: case matters, any byte but
 * '/' and NUL is part of a name. */
#include "a_sys.h"

#define D "/tmp/a_stat_names"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

int test_main(void) {
    struct a_stat a, b;
    const char *names[] = {D "/a", D "/b", D "/c", D "/A", D "/sp ace", D "/\x01\xff", D "/r"};
    for (unsigned i = 0; i < sizeof names / sizeof names[0]; i++) a_unlink(names[i]);
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("make_a", a_make_file(D "/a", "aaaa", 0644), 0);
    st(D "/a", &a);

    expect("link_b", syscall5(SYS_linkat, AT_FDCWD, D "/a", AT_FDCWD, D "/b", 0), 0);
    expect("stat_b", st(D "/b", &b), 0);
    check("link_same_inode", b.st_ino == a.st_ino && b.st_dev == a.st_dev, 0);
    expect("link_nlink_2", b.st_nlink, 2);
    st(D "/a", &b);
    expect("original_nlink_2", b.st_nlink, 2);

    expect("rename_b_c", syscall4(SYS_renameat, AT_FDCWD, D "/b", AT_FDCWD, D "/c"), 0);
    expect("old_name_enoent", st(D "/b", &b), -ENOENT);
    expect("new_name", st(D "/c", &b), 0);
    check("rename_keeps_inode", b.st_ino == a.st_ino, 0);
    expect("rename_keeps_nlink", b.st_nlink, 2);

    expect("make_r", a_make_file(D "/r", "rr", 0600), 0);
    struct a_stat r;
    st(D "/r", &r);
    expect("rename_over_c", syscall4(SYS_renameat, AT_FDCWD, D "/r", AT_FDCWD, D "/c"), 0);
    expect("stat_c_replaced", st(D "/c", &b), 0);
    check("c_is_r_now", b.st_ino == r.st_ino, 0);
    expect("c_size_2", b.st_size, 2);
    st(D "/a", &b);
    expect("a_nlink_back_to_1", b.st_nlink, 1);

    expect("upper_case_enoent", st(D "/A", &b), -ENOENT);
    expect("make_upper", a_make_file(D "/A", "", 0644), 0);
    expect("stat_upper", st(D "/A", &b), 0);
    check("case_distinct_inode", b.st_ino != a.st_ino, 0);
    expect("make_space", a_make_file(D "/sp ace", "", 0644), 0);
    expect("stat_space", st(D "/sp ace", &b), 0);
    expect("make_bytes", a_make_file(D "/\x01\xff", "", 0644), 0);
    expect("stat_bytes", st(D "/\x01\xff", &b), 0);
    expect("stat_bytes_prefix_enoent", st(D "/\x01", &b), -ENOENT);

    for (unsigned i = 0; i < sizeof names / sizeof names[0]; i++) a_unlink(names[i]);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
