/* link(2) errors: EEXIST for any existing newpath (dangling links
 * included), EPERM for a directory oldpath, ENOENT, ENOTDIR, ELOOP,
 * ENAMETOOLONG, EFAULT on either path, trailing slashes; their order. */
#include "a_sys.h"

#define D "/tmp/lb_link_errors"

static i64 link_(const char *a, const char *b) { return syscall2(SYS_link, a, b); }

static char buf[8192], huge[4200];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_g", a_make_file(D "/g", "y", 0644), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nowhere", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);

    expect("exists_file", link_(D "/f", D "/g"), -EEXIST);
    expect("exists_same", link_(D "/f", D "/f"), -EEXIST);
    expect("exists_dir", link_(D "/f", D "/sub"), -EEXIST);
    expect("exists_dangling", link_(D "/f", D "/dangling"), -EEXIST);
    expect("nowhere_not_created", a_lstat_path(D "/nowhere", &st), -ENOENT);
    expect("exists_dot", link_(D "/f", D "/sub/."), -EEXIST);
    expect("exists_dotdot", link_(D "/f", D "/sub/.."), -EEXIST);
    expect("dir_eperm", link_(D "/sub", D "/sub2"), -EPERM);
    expect("dir_slash_eperm", link_(D "/sub/", D "/sub2"), -EPERM);
    expect("root_eperm", link_("/", "/lb_link_errors_never"), -EPERM);
    expect("dot_eperm", link_(D "/sub/.", D "/sub3"), -EPERM);
    expect("missing_old", link_(D "/nope", D "/x"), -ENOENT);
    expect("missing_old_dir", link_(D "/nope/a", D "/x"), -ENOENT);
    expect("missing_new_dir", link_(D "/f", D "/nope/x"), -ENOENT);
    expect("empty_old", link_("", D "/x"), -ENOENT);
    expect("empty_new", link_(D "/f", ""), -ENOENT);
    expect("dangling_old_slash", link_(D "/dangling/", D "/x"), -ENOENT);
    expect("file_slash_old", link_(D "/f/", D "/x"), -ENOTDIR);
    expect("new_trailing_slash", link_(D "/f", D "/x/"), -ENOENT);
    expect("new_trailing_slash_existing", link_(D "/f", D "/g/"), -EEXIST);
    expect("file_as_dir_old", link_(D "/f/x", D "/y"), -ENOTDIR);
    expect("file_as_dir_new", link_(D "/f", D "/g/y"), -ENOTDIR);
    expect("loop_old_slash", link_(D "/loop/", D "/x"), -ELOOP);
    expect("loop_new_dir", link_(D "/f", D "/loop/x"), -ELOOP);
    expect("null_old", link_((const char *)0, D "/x"), -EFAULT);
    expect("null_new", link_(D "/f", (const char *)0), -EFAULT);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'k';
    p[256] = 0;
    expect("long_old", link_(buf, D "/x"), -ENAMETOOLONG);
    expect("long_new", link_(D "/f", buf), -ENAMETOOLONG);
    p[255] = 0;
    expect("name_255_new_ok", link_(D "/f", buf), 0);
    expect("name_255_cleanup", a_unlink(buf), 0);
    p[255] = 'k';
    for (int i = 0; i < 4096; i++) huge[i] = '/';
    huge[4096] = 0;
    expect("huge_old", link_(huge, D "/x"), -ENAMETOOLONG);
    expect("huge_new", link_(D "/f", huge), -ENAMETOOLONG);
    expect("nothing_created", a_lstat_path(D "/x", &st), -ENOENT);

    /* order: oldpath is resolved completely before newpath is looked at */
    expect("order_missing_old_vs_huge_new", link_(D "/missing", huge), -ENOENT);
    expect("order_missing_old_vs_long_new", link_(D "/missing", buf), -ENOENT);
    expect("order_dir_old_vs_missing_new_dir", link_(D "/sub", D "/nope/x"), -ENOENT);
    expect("order_dir_old_vs_existing_new", link_(D "/sub", D "/g"), -EEXIST);
    expect("order_long_old_vs_missing_new", link_(buf, D "/nope/x"), -ENAMETOOLONG);
    expect("order_huge_old_vs_null_new", link_(huge, (const char *)0), -ENAMETOOLONG);

    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
