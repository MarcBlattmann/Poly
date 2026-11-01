/* rename(2) errors: ENOENT (missing source, missing target directory,
 * empty paths), EBUSY for "." or ".." as the last component, ENOTDIR for
 * a trailing slash after a non-directory, ELOOP, ENAMETOOLONG, EFAULT,
 * and which of the two paths is judged first. A failed call changes
 * nothing. */
#include "a_sys.h"

#define D "/tmp/lb_rename_errors"

static i64 rename_(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

static char buf[8192], buf2[8192];

static void long_name(char *out, const char *prefix, const char *suffix) {
    char *p = str_cpy(out, prefix);
    for (int i = 0; i < 256; i++) p[i] = 'n';
    str_cpy(p + 256, suffix);
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_g", a_make_file(D "/g", "y", 0644), 0);
    expect("setup_lfile", syscall2(SYS_symlink, "f", D "/lfile"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);

    expect("missing_source", rename_(D "/nope", D "/x"), -ENOENT);
    expect("missing_source_dir", rename_(D "/nope/a", D "/x"), -ENOENT);
    expect("missing_target_dir", rename_(D "/f", D "/nope/x"), -ENOENT);
    expect("empty_old", rename_("", D "/x"), -ENOENT);
    expect("empty_new", rename_(D "/f", ""), -ENOENT);
    expect("f_kept", a_lstat_path(D "/f", &st), 0);
    expect("source_dot_ebusy", rename_(D "/sub/.", D "/x"), -EBUSY);
    expect("source_dotdot_ebusy", rename_(D "/sub/..", D "/x"), -EBUSY);
    expect("target_dot_ebusy", rename_(D "/f", D "/sub/."), -EBUSY);
    expect("target_dotdot_ebusy", rename_(D "/f", D "/sub/.."), -EBUSY);
    expect("root_source_ebusy", rename_("/", "/lb_rename_errors_never"), -EBUSY);
    expect("file_slash_source", rename_(D "/f/", D "/x"), -ENOTDIR);
    expect("file_slash_target", rename_(D "/f", D "/x/"), -ENOTDIR);
    expect("file_slash_existing_target", rename_(D "/f", D "/g/"), -ENOTDIR);
    expect("link_slash_source", rename_(D "/lfile/", D "/x"), -ENOTDIR);
    expect("file_as_dir_source", rename_(D "/f/x", D "/y"), -ENOTDIR);
    expect("file_as_dir_target", rename_(D "/g", D "/f/y"), -ENOTDIR);
    expect("loop_dir_eloop", rename_(D "/loop/x", D "/y"), -ELOOP);
    expect("loop_target_dir_eloop", rename_(D "/f", D "/loop/y"), -ELOOP);
    expect("null_old_efault", rename_((const char *)0, D "/x"), -EFAULT);
    expect("null_new_efault", rename_(D "/f", (const char *)0), -EFAULT);
    long_name(buf, D "/", "");
    expect("long_old", rename_(buf, D "/x"), -ENAMETOOLONG);
    expect("long_new", rename_(D "/f", buf), -ENAMETOOLONG);
    for (int i = 0; i < 4096; i++) buf2[i] = '/';
    buf2[4096] = 0;
    expect("path_4096_new", rename_(D "/f", buf2), -ENAMETOOLONG);
    expect("f_still_kept", a_lstat_path(D "/f", &st), 0);
    expect("g_still_kept", a_lstat_path(D "/g", &st), 0);

    /* order between the two paths: every directory of both paths first,
     * then the last names */
    expect("order_long_old_vs_missing_newdir", rename_(buf, D "/nope/x"), -ENOENT);
    expect("order_long_old_vs_notdir_newdir", rename_(buf, D "/f/x"), -ENOTDIR);
    expect("order_missing_old_vs_long_new", rename_(D "/missing", buf), -ENOENT);
    expect("order_missing_old_vs_4096_new", rename_(D "/missing", buf2), -ENAMETOOLONG);
    expect("order_missing_olddir_vs_4096_new", rename_(D "/nope/x", buf2), -ENOENT);
    expect("order_missing_olddir_vs_missing_newdir", rename_(D "/nope/x", D "/f/y"), -ENOENT);
    expect("order_old_ok_newdir_missing", rename_(D "/f", D "/nope/x"), -ENOENT);
    long_name(buf, D "/nope/", "");
    expect("order_missing_before_long", rename_(buf, D "/x"), -ENOENT);
    expect("order_dot_vs_missing_target_dir", rename_(D "/sub/.", D "/nope/x"), -ENOENT);
    expect("order_missing_source_vs_dot_target", rename_(D "/missing", D "/sub/."), -EBUSY);

    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_lfile", a_unlink(D "/lfile"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
