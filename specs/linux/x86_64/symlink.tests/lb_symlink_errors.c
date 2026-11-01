/* symlink(2) errors: EEXIST for an existing linkpath (anything, a dangling
 * link included), ENOENT for an empty target or linkpath or a missing
 * directory, ENAMETOOLONG for a target of 4096 bytes or more, ENOTDIR,
 * ELOOP, EFAULT; trailing slashes; order between target and linkpath. */
#include "a_sys.h"

#define D "/tmp/lb_symlink_errors"

static i64 symlink_(const char *t, const char *l) { return syscall2(SYS_symlink, t, l); }

static char buf[8192], huge[4200];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nowhere", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);

    expect("exists_file", symlink_("x", D "/f"), -EEXIST);
    expect("exists_dir", symlink_("x", D "/sub"), -EEXIST);
    expect("exists_dir_slash", symlink_("x", D "/sub/"), -EEXIST);
    expect("exists_dangling", symlink_("x", D "/dangling"), -EEXIST);
    expect("exists_dangling_slash", symlink_("x", D "/dangling/"), -EEXIST);
    expect("nowhere_not_created", a_lstat_path(D "/nowhere", &st), -ENOENT);
    expect("exists_loop", symlink_("x", D "/loop"), -EEXIST);
    expect("exists_dot", symlink_("x", D "/sub/."), -EEXIST);
    expect("exists_dotdot", symlink_("x", D "/sub/.."), -EEXIST);
    expect("exists_root", symlink_("x", "/"), -EEXIST);
    expect("new_trailing_slash", symlink_("x", D "/new/"), -ENOENT);
    expect("new_not_created", a_lstat_path(D "/new", &st), -ENOENT);
    expect("empty_target", symlink_("", D "/e"), -ENOENT);
    expect("empty_linkpath", symlink_("x", ""), -ENOENT);
    expect("missing_dir", symlink_("x", D "/nope/l"), -ENOENT);
    expect("dangling_dir", symlink_("x", D "/dangling/l"), -ENOENT);
    expect("file_as_dir", symlink_("x", D "/f/l"), -ENOTDIR);
    expect("loop_dir", symlink_("x", D "/loop/l"), -ELOOP);
    expect("null_target", symlink_((const char *)0, D "/n"), -EFAULT);
    expect("null_linkpath", symlink_("x", (const char *)0), -EFAULT);
    for (int i = 0; i < 4096; i++) huge[i] = 'h';
    huge[4096] = 0;
    expect("target_4096", symlink_(huge, D "/h"), -ENAMETOOLONG);
    huge[4095] = 0;
    expect("target_4095_ok", symlink_(huge, D "/h"), 0);
    expect("target_4095_cleanup", a_unlink(D "/h"), 0);
    huge[4095] = 'h';
    /* a target with a 256-byte component is fine (never resolved) */
    for (int i = 0; i < 300; i++) buf[i] = 'c';
    buf[300] = 0;
    expect("target_long_component_ok", symlink_(buf, D "/lc"), 0);
    expect("target_long_component_follow", a_stat_path(D "/lc", &st), -ENAMETOOLONG);
    expect("target_long_component_cleanup", a_unlink(D "/lc"), 0);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 's';
    p[256] = 0;
    expect("linkpath_256", symlink_("x", buf), -ENAMETOOLONG);
    p[255] = 0;
    expect("linkpath_255_ok", symlink_("x", buf), 0);
    expect("linkpath_255_cleanup", a_unlink(buf), 0);
    p[255] = 's';

    /* order: the target string is checked first, then the linkpath */
    expect("order_empty_target_vs_existing", symlink_("", D "/f"), -ENOENT);
    expect("order_huge_target_vs_existing", symlink_(huge, D "/f"), -ENAMETOOLONG);
    expect("order_null_target_vs_null_link", symlink_((const char *)0, (const char *)0), -EFAULT);
    expect("order_empty_target_vs_null_link", symlink_("", (const char *)0), -ENOENT);
    expect("order_missing_dir_vs_long_name", symlink_("x", D "/nope/sss"), -ENOENT);
    p = str_cpy(buf, D "/nope/");
    for (int i = 0; i < 256; i++) p[i] = 's';
    p[256] = 0;
    expect("order_missing_before_long", symlink_("x", buf), -ENOENT);

    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
