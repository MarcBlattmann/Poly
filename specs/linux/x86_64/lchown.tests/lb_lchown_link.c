/* lchown(2) does not follow a symbolic link at the end of the path: the
 * link's own uid/gid change (root), the target's do not. A dangling link
 * and a link to itself are fine. Links earlier in the path, and a link
 * followed by a trailing slash, are followed. */
#include "a_sys.h"

#define D "/tmp/lb_lchown_link"
#define SYS_lchown 94

static i64 lchown_(const char *p, i64 u, i64 g) { return syscall3(SYS_lchown, p, u, g); }
static void lids(const char *p, i64 *u, i64 *g) {
    struct a_stat st;
    *u = *g = -1;
    if (a_lstat_path(p, &st) == 0) *u = st.st_uid, *g = st.st_gid;
}

int test_main(void) {
    i64 uid = syscall0(SYS_getuid), gid = syscall0(SYS_getgid);
    int root = uid == 0;
    i64 u, g;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_inner", a_make_file(D "/d/g", "x", 0644), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    expect("setup_dirlink", syscall2(SYS_symlink, "d", D "/ld"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "missing", D "/dangling"), 0);
    expect("setup_self", syscall2(SYS_symlink, "self", D "/self"), 0);

    i64 nu = root ? 501 : uid, ng = root ? 502 : gid;
    expect("link_lchown", lchown_(D "/l", nu, ng), 0);
    lids(D "/l", &u, &g);
    expect("link_uid", u, nu);
    expect("link_gid", g, ng);
    lids(D "/f", &u, &g);
    expect("target_uid_unchanged", u, uid);
    expect("target_gid_unchanged", g, gid);
    expect("link_mode_still_0777", a_lstat_path(D "/l", &st) == 0 && st.st_mode == (S_IFLNK | 0777), 1);
    expect("link_give_away", lchown_(D "/l", 4321, -1), root ? 0 : -EPERM);

    expect("dangling_ok", lchown_(D "/dangling", -1, ng), 0);
    lids(D "/dangling", &u, &g);
    expect("dangling_gid", g, ng);
    expect("self_loop_ok", lchown_(D "/self", nu, -1), 0);
    lids(D "/self", &u, &g);
    expect("self_uid", u, nu);

    /* a trailing slash makes the link be followed: the directory changes */
    expect("dirlink_slash", lchown_(D "/ld/", nu, ng), 0);
    lids(D "/d", &u, &g);
    expect("dir_uid_via_slash", u, nu);
    expect("dir_gid_via_slash", g, ng);
    lids(D "/ld", &u, &g);
    expect("dirlink_itself_unchanged", u, uid);
    /* a link in the middle is followed */
    expect("through_dirlink", lchown_(D "/ld/g", nu, ng), 0);
    lids(D "/d/g", &u, &g);
    expect("inner_uid", u, nu);
    /* a link with a trailing slash to a file: ENOTDIR; dangling with slash: ENOENT */
    expect("filelink_slash_enotdir", lchown_(D "/l/", -1, -1), -ENOTDIR);
    expect("dangling_slash_enoent", lchown_(D "/dangling/", -1, -1), -ENOENT);
    expect("self_slash_eloop", lchown_(D "/self/", -1, -1), -ELOOP);

    /* on a regular file lchown acts like chown */
    expect("file_lchown", lchown_(D "/f", nu, ng), 0);
    lids(D "/f", &u, &g);
    expect("file_uid", u, nu);
    expect("file_chmod_suid", syscall2(SYS_chmod, D "/f", 04755), 0);
    expect("file_lchown_noop", lchown_(D "/f", -1, -1), 0);
    expect("file_suid_cleared", a_stat_path(D "/f", &st) == 0 && (st.st_mode & 07777) == 0755, 1);

    expect("cleanup_self", a_unlink(D "/self"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_ld", a_unlink(D "/ld"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_inner", a_unlink(D "/d/g"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
