/* symlink: symbolic links in the middle of the linkpath are followed,
 * the last component never is. Through a link to a directory the new link
 * is made in that directory; through a dangling link ENOENT; through a
 * link to a regular file ENOTDIR; through a link to itself ELOOP. At most
 * 40 links are followed in one lookup: a chain of 40 works, 41 is ELOOP,
 * both for open and for a directory in a linkpath. */
#include "a_sys.h"

#define D "/tmp/c_symlink_follow"

/* buf = D "/" prefix number */
static char *name_of(char *buf, const char *prefix, int n, int full) {
    char *q = buf;
    if (full)
        for (const char *s = D "/"; *s; s++) *q++ = *s;
    for (; *prefix; prefix++) *q++ = *prefix;
    a_utoa(q, (u64)n);
    return buf;
}

int test_main(void) {
    struct a_stat st;
    char a[64], b[64];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("setup_ldir", syscall2(SYS_symlink, "sub", D "/ldir"), 0);
    expect("setup_labs", syscall2(SYS_symlink, D "/sub", D "/labs"), 0);
    expect("setup_lfile", syscall2(SYS_symlink, "f", D "/lfile"), 0);
    expect("setup_dang", syscall2(SYS_symlink, "nowhere", D "/dang"), 0);
    expect("setup_self", syscall2(SYS_symlink, "self", D "/self"), 0);

    expect("through_dirlink", syscall2(SYS_symlink, "x", D "/ldir/in"), 0);
    expect("made_in_target_dir", a_lstat_path(D "/sub/in", &st), 0);
    expect("made_is_link", st.st_mode & S_IFMT, S_IFLNK);
    expect("through_abs_dirlink", syscall2(SYS_symlink, "x", D "/labs/in2"), 0);
    expect("made_in_target_dir2", a_lstat_path(D "/sub/in2", &st), 0);
    expect("through_dangling_enoent", syscall2(SYS_symlink, "x", D "/dang/x"), -ENOENT);
    expect("dangling_target_not_made", a_lstat_path(D "/nowhere", &st), -ENOENT);
    expect("through_filelink_enotdir", syscall2(SYS_symlink, "x", D "/lfile/x"), -ENOTDIR);
    expect("through_self_eloop", syscall2(SYS_symlink, "x", D "/self/x"), -ELOOP);
    expect("dirlink_dotdot", syscall2(SYS_symlink, "x", D "/ldir/../top"), 0);
    expect("dotdot_after_link_is_parent_of_target", a_lstat_path(D "/top", &st), 0);

    /* c0 -> f, c<n> -> c<n-1>: opening c<n> follows n + 1 links. */
    expect("setup_c0", syscall2(SYS_symlink, "f", name_of(a, "c", 0, 1)), 0);
    int ok = 1;
    for (int i = 1; i <= 40; i++) ok &= syscall2(SYS_symlink, name_of(b, "c", i - 1, 0), name_of(a, "c", i, 1)) == 0;
    check("setup_chain", ok, 0);
    i64 fd = a_open(name_of(a, "c", 39, 1), O_RDONLY, 0);
    check("open_40_links_ok", fd >= 0, fd);
    a_close(fd);
    expect("open_41_links_eloop", a_open(name_of(a, "c", 40, 1), O_RDONLY, 0), -ELOOP);

    /* e0 -> sub, e<n> -> e<n-1>: e<n>/name follows n + 1 links. */
    expect("setup_e0", syscall2(SYS_symlink, "sub", name_of(a, "e", 0, 1)), 0);
    ok = 1;
    for (int i = 1; i <= 40; i++) ok &= syscall2(SYS_symlink, name_of(b, "e", i - 1, 0), name_of(a, "e", i, 1)) == 0;
    check("setup_dir_chain", ok, 0);
    expect("linkpath_40_links_ok", syscall2(SYS_symlink, "x", D "/e39/deep"), 0);
    expect("made_at_chain_end", a_lstat_path(D "/sub/deep", &st), 0);
    expect("linkpath_41_links_eloop", syscall2(SYS_symlink, "x", D "/e40/deeper"), -ELOOP);

    ok = 1;
    for (int i = 0; i <= 40; i++) ok &= a_unlink(name_of(a, "c", i, 1)) == 0;
    for (int i = 0; i <= 40; i++) ok &= a_unlink(name_of(a, "e", i, 1)) == 0;
    const char *names[] = {D "/sub/in", D "/sub/in2", D "/sub/deep", D "/top", D "/ldir",
                           D "/labs",   D "/lfile",   D "/dang",     D "/self", D "/f"};
    for (int i = 0; i < 10; i++) ok &= a_unlink(names[i]) == 0;
    ok &= a_rmdir(D "/sub") == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
