/* What a redirect rename (redirect_dir=on) does around the directory. A
 * directory that was copied up before (it has upper content and whiteouts
 * of its own) is moved as it is, with the redirect and a whiteout at the old
 * name; one that only exists in the upper layer gets neither. Renaming back
 * to the original name leaves the redirect (to its own name) and no whiteout.
 * A directory made at the old name is empty (it is marked opaque, the
 * whiteout it replaces is gone); removing it brings the whiteout back. A
 * directory renamed onto the old name of another one replaces the whiteout and
 * shows only its own content. A target: an empty lower directory is replaced,
 * a non-empty one is ENOTEMPTY, a file ENOTDIR, a directory onto itself
 * EINVAL. RENAME_EXCHANGE of a lower directory with a file or with another
 * directory works (each gets a redirect to the other's name), RENAME_NOREPLACE
 * to a free name works, onto a name is EEXIST. A renamed directory can only be
 * removed when it is empty in the merged view. Two lower layers: both
 * directories of the name move together.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN; trusted.* attributes)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovc_rmerge"
#define M B "/m"
#define U B "/u"
#define L B "/l"
#define L2 B "/l2"
#define RENAME_NOREPLACE_ 1
#define RENAME_EXCHANGE_ 2

static int whiteout(const char *p) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 && (st.st_mode & S_IFMT) == S_IFCHR && st.st_rdev == 0;
}

static i64 ren(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }
static i64 ren2(const char *a, const char *b, i64 flags) {
    return syscall5(SYS_renameat2, AT_FDCWD, a, AT_FDCWD, b, flags);
}

static void xattr_is(const char *name, const char *path, const char *attr, const char *want) {
    char v[64];
    memset(v, 0, sizeof v);
    i64 r = syscall4(SYS_lgetxattr, path, attr, v, sizeof v - 1);
    if (!want) {
        expect(name, r, -ENODATA);
        return;
    }
    check(name, r == (i64)str_len(want) && str_eq(v, want), r);
}
#define REDIR "trusted.overlay.redirect"
#define OPAQUE "trusted.overlay.opaque"

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    pm_mkdirs(L "/dir");
    pm_mkdirs(L "/b");
    pm_mkdirs(L "/c");
    pm_mkdirs(L "/o1");
    pm_mkdirs(L "/o2");
    pm_mkdirs(L "/t1");
    pm_mkdirs(L "/t2");
    pm_mkdirs(L "/t3");
    pm_mkdirs(L "/t4");
    pm_mkdirs(L "/xa");
    pm_mkdirs(L "/xb");
    pm_mkdirs(L "/xc");
    pm_mkdirs(L "/r");
    pm_mkdirs(L "/two");
    pm_mkdirs(L "/only1");
    pm_mkdirs(L2 "/two");
    pm_mkdirs(L2 "/only2");
    pm_mkdirs(U);
    pm_mkdirs(B "/w");
    pm_mkdirs(M);
    pm_put(L, "/dir/f", "df", 0644);
    pm_put(L, "/dir/g", "dg", 0644);
    pm_put(L, "/b/f", "bf", 0644);
    pm_put(L, "/c/f", "cf", 0644);
    pm_put(L, "/o1/x", "ox", 0644);
    pm_put(L, "/o2/y", "oy", 0644);
    pm_put(L, "/t2/z", "tz", 0644);
    pm_put(L, "/t3/k", "tk", 0644);
    pm_put(L, "/xa/a", "xa", 0644);
    pm_put(L, "/xb/b", "xb", 0644);
    pm_put(L, "/xc/c", "xc", 0644);
    pm_put(L, "/file", "plain", 0644);
    pm_put(L, "/r/f", "rf", 0644);
    pm_put(L, "/two/one", "1", 0644);
    pm_put(L2, "/two/two", "2", 0644);
    pm_put(L, "/only1/o", "o", 0644);
    pm_put(L2, "/only2/p", "p", 0644);

    const char *on = "lowerdir=" L ":" L2 ",upperdir=" U ",workdir=" B "/w,redirect_dir=on,index=off,metacopy=off";
    expect("mount", pm_mount("ov", M, "overlay", 0, on), 0);

    /* a directory copied up before: moved as it is */
    expect("copy_up_dir", a_make_file(M "/dir/new", "n", 0644), 0);
    expect("whiteout_in_dir", syscall1(SYS_unlink, M "/dir/g"), 0);
    expect("whiteout_made", whiteout(U "/dir/g"), 1);
    expect("rename_merged_dir", ren(M "/dir", M "/moved"), 0);
    xattr_is("moved_redirect", U "/moved", REDIR, "dir");
    expect("dir_whiteout", whiteout(U "/dir"), 1);
    expect("inner_whiteout_moved", whiteout(U "/moved/g"), 1);
    expect("inner_file_moved", pm_exists(U "/moved/new"), 1);
    pm_expect_list("moved_listing", M "/moved", "f,new");
    pm_expect_file("moved_new", M "/moved/new", "n");
    expect("dir_gone", pm_exists(M "/dir"), 0);

    /* a directory made in the merged view (upper only): neither */
    expect("mkdir_up", a_mkdir(M "/up", 0755), 0);
    expect("mkdir_up_sub", a_mkdir(M "/up/sub", 0755), 0);
    expect("up_file", a_make_file(M "/up/sub/in", "inside", 0644), 0);
    expect("rename_up", ren(M "/up", M "/up2"), 0);
    xattr_is("up2_no_redirect", U "/up2", REDIR, 0);
    expect("up_nothing_left", pm_exists(U "/up"), 0);
    pm_expect_file("up2_content", M "/up2/sub/in", "inside");

    /* back to the original name */
    expect("b_out", ren(M "/b", M "/bm"), 0);
    expect("b_back", ren(M "/bm", M "/b"), 0);
    xattr_is("b_redirect_self", U "/b", REDIR, "b");
    expect("b_no_whiteout", whiteout(U "/b"), 0);
    expect("b_no_bm", pm_exists(U "/bm"), 0);
    pm_expect_file("b_content", M "/b/f", "bf");

    /* a directory at the old name: empty, opaque; removing it, whiteout again */
    expect("c_out", ren(M "/c", M "/cm"), 0);
    expect("c_new", a_mkdir(M "/c", 0755), 0);
    pm_expect_list("c_new_empty", M "/c", "");
    xattr_is("c_new_opaque", U "/c", OPAQUE, "y");
    xattr_is("c_new_no_redirect", U "/c", REDIR, 0);
    pm_expect_list("cm_listing", M "/cm", "f");
    expect("c_new_rmdir", a_rmdir(M "/c"), 0);
    expect("c_whiteout_again", whiteout(U "/c"), 1);
    pm_expect_list("c_gone_listing", M "/cm", "f");
    expect("c_gone", pm_exists(M "/c"), 0);

    /* a directory renamed onto the name of a renamed one */
    expect("o1_out", ren(M "/o1", M "/o1m"), 0);
    expect("o2_onto_o1", ren(M "/o2", M "/o1"), 0);
    xattr_is("o1_redirect", U "/o1", REDIR, "o2");
    expect("o2_whiteout", whiteout(U "/o2"), 1);
    pm_expect_list("o1_listing", M "/o1", "y");
    pm_expect_list("o1m_listing", M "/o1m", "x");
    xattr_is("o1_hidden", M "/o1", REDIR, 0);

    /* targets */
    expect("t4_onto_empty_lower", ren(M "/t4", M "/t1"), 0);
    xattr_is("t1_redirect", U "/t1", REDIR, "t4");
    expect("t4_whiteout", whiteout(U "/t4"), 1);
    expect("t3_onto_nonempty", ren(M "/t3", M "/t2"), -ENOTEMPTY);
    pm_expect_file("t2_unchanged", M "/t2/z", "tz");
    pm_expect_file("t3_unchanged", M "/t3/k", "tk");
    expect("dir_onto_file", ren(M "/t3", M "/file"), -ENOTDIR);
    expect("file_onto_dir", ren(M "/file", M "/t3"), -EISDIR);
    expect("dir_into_itself", ren(M "/t3", M "/t3/sub"), -EINVAL);

    /* flags */
    expect("noreplace_existing", ren2(M "/xa", M "/xb", RENAME_NOREPLACE_), -EEXIST);
    expect("noreplace_free", ren2(M "/xa", M "/xa2", RENAME_NOREPLACE_), 0);
    xattr_is("xa2_redirect", U "/xa2", REDIR, "xa");
    pm_expect_file("xa2_content", M "/xa2/a", "xa");
    expect("exchange_dirs", ren2(M "/xb", M "/xc", RENAME_EXCHANGE_), 0);
    pm_expect_list("xb_has_c", M "/xb", "c");
    pm_expect_list("xc_has_b", M "/xc", "b");
    xattr_is("xb_redirect", U "/xb", REDIR, "xc");
    xattr_is("xc_redirect", U "/xc", REDIR, "xb");
    expect("exchange_dir_file", ren2(M "/xa2", M "/file", RENAME_EXCHANGE_), 0);
    expect("file_now_dir_name", pm_mode(M "/xa2") & S_IFMT, S_IFREG);
    pm_expect_file("xa2_is_file", M "/xa2", "plain");
    pm_expect_list("file_is_dir", M "/file", "a");
    xattr_is("file_redirect", U "/file", REDIR, "xa");
    expect("exchange_with_missing", ren2(M "/xb", M "/nothere", RENAME_EXCHANGE_), -ENOENT);

    /* removing a renamed directory */
    expect("r_out", ren(M "/r", M "/rm"), 0);
    expect("rmdir_nonempty", a_rmdir(M "/rm"), -ENOTEMPTY);
    expect("rm_child", syscall1(SYS_unlink, M "/rm/f"), 0);
    expect("rmdir_emptied", a_rmdir(M "/rm"), 0);
    expect("rm_gone", pm_exists(M "/rm"), 0);
    expect("r_still_whiteout", whiteout(U "/r"), 1);
    expect("rm_not_in_upper", pm_exists(U "/rm"), 0);

    /* two lower layers: both directories of the name move together */
    pm_expect_list("two_listing", M "/two", "one,two");
    expect("two_out", ren(M "/two", M "/twom"), 0);
    pm_expect_list("twom_listing", M "/twom", "one,two");
    xattr_is("twom_redirect", U "/twom", REDIR, "two");
    expect("two_gone", pm_exists(M "/two"), 0);
    expect("only1_out", ren(M "/only1", M "/only1m"), 0);
    expect("only2_out", ren(M "/only2", M "/only2m"), 0);
    pm_expect_file("only2_content", M "/only2m/p", "p");
    xattr_is("only2_redirect", U "/only2m", REDIR, "only2");

    /* a directory made in a directory that exists below is marked opaque */
    expect("mkdir_fresh", a_mkdir(M "/fresh", 0755), 0);
    xattr_is("fresh_opaque", U "/fresh", OPAQUE, "y");
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
