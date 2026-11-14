/* What redirect_dir= decides. Renaming a directory that exists in a lower
 * layer needs redirect_dir=on: with off, follow and nofollow the rename (and a
 * RENAME_EXCHANGE or a RENAME_NOREPLACE to a free name) is EXDEV, and nothing
 * is changed; a directory that only exists in the upper layer can always be
 * renamed (without a redirect), also exchanged with another one. An unknown
 * value (and an empty one) is EINVAL at mount time. Redirect attributes that
 * are already in the layers are followed under follow and on: the directory
 * shows the content of the lower directory the redirect names (relative: a
 * sibling, absolute: from the root, looked up in the layers below the one
 * that holds the attribute; it is not merged with a directory of that name
 * above); under nofollow the entry cannot be looked up at all (EPERM) while
 * its siblings and the listing of its parent are fine. A redirect that names
 * nothing gives an empty directory.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN; trusted.* attributes)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_ovc_rmodes"
#define RENAME_NOREPLACE_ 1
#define RENAME_EXCHANGE_ 2

static i64 ren2(const char *a, const char *b, i64 flags) {
    return syscall5(SYS_renameat2, AT_FDCWD, a, AT_FDCWD, b, flags);
}

static void cat(char *o, const char *a, const char *b, const char *c) {
    size_t n = 0;
    while (*a) o[n++] = *a++;
    while (*b) o[n++] = *b++;
    while (*c) o[n++] = *c++;
    o[n] = 0;
}

static i64 set_redirect(const char *p, const char *v) {
    return syscall5(SYS_lsetxattr, p, "trusted.overlay.redirect", v, str_len(v), 0);
}

/* the creating modes: nothing but redirect_dir=on renames a lower directory */
static void creating_mode(const char *tag, const char *opt) {
    char l[128], u[128], w[128], m[128], d[128], o[512], n[96];
    cat(d, B "/", tag, "");
    cat(l, d, "/l", "");
    cat(u, d, "/u", "");
    cat(w, d, "/w", "");
    cat(m, d, "/m", "");
    pm_mkdirs(l);
    pm_mkdirs(u);
    pm_mkdirs(w);
    pm_mkdirs(m);
    char p[160], q[160], t[160];
    cat(p, l, "/d1", "");
    pm_mkdirs(p);
    pm_put(l, "/d1/f", "one", 0644);
    cat(p, l, "/d2", "");
    pm_mkdirs(p);
    cat(p, l, "/e1", "");
    pm_mkdirs(p);
    cat(p, l, "/e2", "");
    pm_mkdirs(p);
    cat(p, l, "/x1", "");
    pm_mkdirs(p);
    cat(p, l, "/x2", "");
    pm_mkdirs(p);
    pm_put(l, "/x1/a", "a", 0644);
    pm_put(l, "/x2/b", "b", 0644);
    pm_put(l, "/plain", "file", 0644);
    cat(o, "lowerdir=", l, ",upperdir=");
    cat(o + str_len(o), u, ",workdir=", w);
    cat(o + str_len(o), ",index=off,metacopy=off,", opt, "");
    cat(n, "mount_", tag, "");
    expect(n, pm_mount("ov", m, "overlay", 0, o), 0);

    cat(p, m, "/d1", "");
    cat(q, m, "/moved", "");
    cat(n, "rename_exdev_", tag, "");
    expect(n, ren2(p, q, 0), -EXDEV);
    cat(t, m, "/d1/f", "");
    cat(n, "rename_left_it_", tag, "");
    expect(n, pm_exists(t), 1);
    cat(t, u, "/moved", "");
    cat(n, "rename_made_nothing_", tag, "");
    expect(n, pm_exists(t), 0);
    cat(n, "upper_empty_", tag, "");
    pm_expect_list(n, u, "");

    cat(p, m, "/e1", "");
    cat(q, m, "/e2", "");
    cat(n, "noreplace_existing_", tag, "");
    expect(n, ren2(p, q, RENAME_NOREPLACE_), -EEXIST);
    cat(q, m, "/e3", "");
    cat(n, "noreplace_free_", tag, "");
    expect(n, ren2(p, q, RENAME_NOREPLACE_), -EXDEV);

    cat(p, m, "/x1", "");
    cat(q, m, "/x2", "");
    cat(n, "exchange_dirs_", tag, "");
    expect(n, ren2(p, q, RENAME_EXCHANGE_), -EXDEV);
    cat(q, m, "/plain", "");
    cat(n, "exchange_dir_file_", tag, "");
    expect(n, ren2(p, q, RENAME_EXCHANGE_), -EXDEV);
    cat(n, "x1_untouched_", tag, "");
    cat(t, m, "/x1/a", "");
    pm_expect_file(n, t, "a");

    /* a directory that exists only in the upper layer */
    cat(p, m, "/up1", "");
    cat(n, "mkdir_up_", tag, "");
    expect(n, a_mkdir(p, 0755), 0);
    cat(t, m, "/up1/in", "");
    a_make_file(t, "inside", 0644);
    cat(q, m, "/up2", "");
    cat(n, "rename_upper_only_", tag, "");
    expect(n, ren2(p, q, 0), 0);
    cat(t, m, "/up2/in", "");
    cat(n, "upper_only_moved_", tag, "");
    pm_expect_file(n, t, "inside");
    cat(t, u, "/up1", "");
    cat(n, "upper_only_leaves_nothing_", tag, "");
    expect(n, pm_exists(t), 0);
    char v[32];
    memset(v, 0, sizeof v);
    cat(t, u, "/up2", "");
    cat(n, "upper_only_no_redirect_", tag, "");
    expect(n, syscall4(SYS_lgetxattr, t, "trusted.overlay.redirect", v, sizeof v), -ENODATA);
    cat(p, m, "/up2", "");
    cat(n, "exchange_upper_dirs_", tag, "");
    cat(t, m, "/up4", "");
    a_mkdir(t, 0755);
    expect(n, ren2(p, t, RENAME_EXCHANGE_), 0);
    expect("tidy", pm_umount(m, 0), 0);
}

/* redirect attributes that are already there */
static void planted(const char *opt, int follows) {
    char o[512];
    pm_rm_rf(B "/pl");
    pm_mkdirs(B "/pl/l1");
    pm_mkdirs(B "/pl/l2/dir/sub");
    pm_mkdirs(B "/pl/l1/dir");
    pm_mkdirs(B "/pl/l1/rel");
    pm_mkdirs(B "/pl/l1/abs");
    pm_mkdirs(B "/pl/l1/none");
    pm_mkdirs(B "/pl/m");
    pm_put(B "/pl/l2", "/dir/f", "from-l2", 0644);
    pm_put(B "/pl/l2", "/dir/sub/s", "sub", 0644);
    pm_put(B "/pl/l1", "/dir/own", "from-l1", 0644);
    expect("plant_rel", set_redirect(B "/pl/l1/rel", "dir"), 0);
    expect("plant_abs", set_redirect(B "/pl/l1/abs", "/dir"), 0);
    expect("plant_none", set_redirect(B "/pl/l1/none", "nothere"), 0);
    pm_mkdirs(B "/pl/u");
    pm_mkdirs(B "/pl/w");
    cat(o, "lowerdir=" B "/pl/l1:" B "/pl/l2,upperdir=" B "/pl/u,workdir=" B "/pl/w,index=off,metacopy=off,", opt, "");
    expect("mount_planted", pm_mount("ov", B "/pl/m", "overlay", 0, o), 0);
    pm_expect_list("planted_root", B "/pl/m", "abs,dir,none,rel");
    pm_expect_list("planted_dir_merged", B "/pl/m/dir", "f,own,sub");
    struct a_stat st;
    if (follows) {
        pm_expect_list("rel_listing", B "/pl/m/rel", "f,sub");
        pm_expect_list("abs_listing", B "/pl/m/abs", "f,sub");
        pm_expect_file("rel_file", B "/pl/m/rel/f", "from-l2");
        pm_expect_file("abs_deep", B "/pl/m/abs/sub/s", "sub");
        expect("rel_is_dir", pm_mode(B "/pl/m/rel") & S_IFMT, S_IFDIR);
        pm_expect_list("none_empty", B "/pl/m/none", "");
        expect("rel_not_merged_with_own", pm_exists(B "/pl/m/rel/own"), 0);
    } else {
        expect("rel_eperm", a_lstat_path(B "/pl/m/rel", &st), -EPERM);
        expect("abs_eperm", a_lstat_path(B "/pl/m/abs", &st), -EPERM);
        expect("none_eperm", a_lstat_path(B "/pl/m/none", &st), -EPERM);
        expect("rel_file_eperm", a_lstat_path(B "/pl/m/rel/f", &st), -EPERM);
        expect("dir_fine", a_lstat_path(B "/pl/m/dir", &st), 0);
        expect("rename_eperm", ren2(B "/pl/m/rel", B "/pl/m/rel2", 0), -EPERM);
        expect("rename_other_exdev", ren2(B "/pl/m/dir", B "/pl/m/dir2", 0), -EXDEV);
    }
    expect("umount_planted", pm_umount(B "/pl/m", 0), 0);
}

static int body(void) {
    expect("tmpfs", pm_tmpfs(B), 0);
    creating_mode("off", "redirect_dir=off");
    creating_mode("follow", "redirect_dir=follow");
    creating_mode("nofollow", "redirect_dir=nofollow");

    /* values that are not modes */
    pm_mkdirs(B "/bad/l");
    pm_mkdirs(B "/bad/u");
    pm_mkdirs(B "/bad/w");
    pm_mkdirs(B "/bad/m");
    expect("value_maybe", pm_mount("ov", B "/bad/m", "overlay", 0, "lowerdir=" B "/bad/l,upperdir=" B "/bad/u,workdir=" B "/bad/w,redirect_dir=maybe"), -EINVAL);
    expect("value_empty", pm_mount("ov", B "/bad/m", "overlay", 0, "lowerdir=" B "/bad/l,upperdir=" B "/bad/u,workdir=" B "/bad/w,redirect_dir="), -EINVAL);
    expect("value_On", pm_mount("ov", B "/bad/m", "overlay", 0, "lowerdir=" B "/bad/l,upperdir=" B "/bad/u,workdir=" B "/bad/w,redirect_dir=On"), -EINVAL);

    planted("redirect_dir=follow", 1);
    planted("redirect_dir=on", 1);
    planted("redirect_dir=nofollow", 0);
    expect("umount_tmpfs", pm_umount(B, 0), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B);
    pm_in_ns(body);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
