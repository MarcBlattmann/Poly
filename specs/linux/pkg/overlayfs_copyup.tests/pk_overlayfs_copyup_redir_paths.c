/* The value of trusted.overlay.redirect after renaming a lower directory
 * with redirect_dir=on. In the same parent it is the old name alone; in
 * another parent it is the old path from the overlay's root, "/a/b/c". It
 * always names the lower directory the content comes from: renaming a
 * redirected directory again keeps the original (a relative value stays
 * relative while the directory stays in its parent, and becomes the original
 * path from the root when it moves to another directory, even when it moves
 * back into the first parent from elsewhere). A directory below a renamed
 * directory that is renamed too gets its old name alone (same parent) or the
 * path it had in the lower layer ("/dir/sub2"). The old names are whiteouts
 * in the upper layer, the content of the lower layer stays. An absolute value
 * of more than 256 bytes (the leading "/" counted) is not made: the rename
 * is EXDEV and nothing changes; a relative one is a name, at most 255 bytes.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN; trusted.* attributes)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovc_rpaths"
#define M B "/m"
#define U B "/u"
#define L B "/l"

static int whiteout(const char *p) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 && (st.st_mode & S_IFMT) == S_IFCHR && st.st_rdev == 0;
}

static i64 ren(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

/* want == 0: the attribute is not there */
static void redir(const char *name, const char *path, const char *want) {
    char v[300];
    memset(v, 0, sizeof v);
    i64 r = syscall4(SYS_lgetxattr, path, "trusted.overlay.redirect", v, sizeof v - 1);
    if (!want) {
        expect(name, r, -ENODATA);
        return;
    }
    check(name, r == (i64)str_len(want) && str_eq(v, want), r);
}

static void fill(char *out, char c, int n) {
    for (int i = 0; i < n; i++) out[i] = c;
    out[n] = 0;
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L "/top1");
    pm_mkdirs(L "/a/b/c");
    pm_mkdirs(L "/a/b/e");
    pm_mkdirs(L "/a/x");
    pm_mkdirs(L "/p/q");
    pm_mkdirs(L "/sub");
    pm_mkdirs(L "/n/sub/deep");
    pm_mkdirs(L "/n/sub2");
    pm_mkdirs(L "/lim");
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    pm_put(L, "/top1/t", "top", 0644);
    pm_put(L, "/a/b/c/g", "gg", 0644);
    pm_put(L, "/a/b/e/h", "hh", 0644);
    pm_put(L, "/p/q/pf", "pf", 0644);
    pm_put(L, "/n/sub/deep/f", "deepf", 0644);
    pm_put(L, "/n/sub2/s2", "s2", 0644);

    const char *on = "lowerdir=" L ",upperdir=" U ",workdir=" B "/w,redirect_dir=on,index=off,metacopy=off";
    expect("mount", pm_mount("ov", M, "overlay", 0, on), 0);

    /* same parent: the old name */
    expect("rename_c", ren(M "/a/b/c", M "/a/b/c2"), 0);
    redir("c_redirect", U "/a/b/c2", "c");
    expect("c_old_whiteout", whiteout(U "/a/b/c"), 1);
    pm_expect_list("b_listing", M "/a/b", "c2,e");
    pm_expect_file("c2_content", M "/a/b/c2/g", "gg");

    /* another parent: the old path from the root */
    expect("rename_e", ren(M "/a/b/e", M "/a/x/e"), 0);
    redir("e_redirect", U "/a/x/e", "/a/b/e");
    expect("e_old_whiteout", whiteout(U "/a/b/e"), 1);
    pm_expect_file("e_content", M "/a/x/e/h", "hh");
    pm_expect_list("x_listing", M "/a/x", "e");

    /* to the top level (the old path has two parts) */
    expect("rename_q", ren(M "/p/q", M "/q"), 0);
    redir("q_redirect", U "/q", "/p/q");
    pm_expect_list("p_listing", M "/p", "");
    pm_expect_list("root_listing", M, "a,lim,n,p,q,sub,top1");

    /* renaming a renamed directory: still the original */
    expect("c2_to_top", ren(M "/a/b/c2", M "/top2"), 0);
    redir("c2_top_redirect", U "/top2", "/a/b/c");
    expect("top2_into_a", ren(M "/top2", M "/a/c3"), 0);
    redir("top2_a_redirect", U "/a/c3", "/a/b/c");
    pm_expect_file("c3_content", M "/a/c3/g", "gg");
    expect("top2_left_nothing", pm_exists(U "/top2"), 0);

    /* relative stays relative in the same parent, goes absolute elsewhere */
    expect("t1_a", ren(M "/top1", M "/top1b"), 0);
    redir("t1_a_redirect", U "/top1b", "top1");
    expect("t1_b", ren(M "/top1b", M "/top1c"), 0);
    redir("t1_b_redirect", U "/top1c", "top1");
    expect("t1_c", ren(M "/top1c", M "/sub/top1d"), 0);
    redir("t1_c_redirect", U "/sub/top1d", "/top1");
    expect("t1_d", ren(M "/sub/top1d", M "/sub/top1e"), 0);
    redir("t1_d_redirect", U "/sub/top1e", "/top1");
    expect("t1_e", ren(M "/sub/top1e", M "/top1f"), 0);
    redir("t1_e_redirect", U "/top1f", "/top1");
    pm_expect_file("t1_content", M "/top1f/t", "top");
    expect("t1_old_gone", pm_exists(M "/top1"), 0);

    /* a directory below a renamed directory */
    expect("rename_n", ren(M "/n", M "/nm"), 0);
    redir("n_redirect", U "/nm", "n");
    expect("rename_sub_same", ren(M "/nm/sub", M "/nm/sub3"), 0);
    redir("sub3_redirect", U "/nm/sub3", "sub");
    expect("rename_sub2_out", ren(M "/nm/sub2", M "/out2"), 0);
    redir("sub2_redirect", U "/out2", "/n/sub2");
    pm_expect_list("nm_listing", M "/nm", "sub3");
    pm_expect_file("deep_content", M "/nm/sub3/deep/f", "deepf");
    pm_expect_file("out2_content", M "/out2/s2", "s2");
    expect("sub_whiteout_in_moved", whiteout(U "/nm/sub"), 1);
    expect("sub2_whiteout_in_moved", whiteout(U "/nm/sub2"), 1);
    expect("n_whiteout", whiteout(U "/n"), 1);

    /* the longest absolute value: 256 bytes with the leading slash */
    char name[300], from[600], to[600];
    fill(name, 'n', 251);
    str_cat2(from, L "/lim/", name);
    expect("make_long_dir", a_mkdir(from, 0755), 0);
    str_cat2(from, M "/lim/", name);
    expect("rename_256", ren(from, M "/m256"), 0);
    char v[300];
    memset(v, 0, sizeof v);
    expect("redirect_256_len", syscall4(SYS_lgetxattr, U "/m256", "trusted.overlay.redirect", v, sizeof v - 1), 256);
    check("redirect_256_value", v[0] == '/' && v[1] == 'l' && v[3] == 'm' && v[4] == '/' && v[5] == 'n' && v[255] == 'n' && v[256] == 0, v[255]);
    fill(name, 'm', 252);
    str_cat2(to, L "/lim/", name);
    expect("make_longer_dir", a_mkdir(to, 0755), 0);
    str_cat2(to, M "/lim/", name);
    expect("rename_257_exdev", ren(to, M "/m257"), -EXDEV);
    expect("rename_257_nothing", pm_exists(U "/m257"), 0);
    expect("rename_257_still_there", pm_exists(to), 1);
    /* the same path split over a shorter absolute value is fine again */
    expect("rename_257_sibling", ren(to, M "/lim/sib"), 0);
    redir("sibling_relative", U "/lim/sib", name);

    /* a relative value is a name: the longest name (255) works in one parent */
    char rn[300], qn[300];
    fill(rn, 'r', 255);
    fill(qn, 'q', 255);
    str_cat2(from, L "/", rn);
    expect("make_name_255", a_mkdir(from, 0755), 0);
    str_cat2(from, M "/", rn);
    str_cat2(to, M "/", qn);
    expect("rename_255", ren(from, to), 0);
    str_cat2(to, U "/", qn);
    memset(v, 0, sizeof v);
    expect("redirect_255_len", syscall4(SYS_lgetxattr, to, "trusted.overlay.redirect", v, sizeof v - 1), 255);
    check("redirect_255_value", str_eq(v, rn), v[0]);
    expect("umount", pm_umount(M, 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
