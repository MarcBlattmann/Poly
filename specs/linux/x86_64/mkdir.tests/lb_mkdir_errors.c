/* mkdir(2) errors: EEXIST for any existing name (file, directory, symbolic
 * link, even a dangling link: it is not followed), ".", ".." and "/";
 * ENOENT for a missing parent or the empty path; ENOTDIR, ELOOP,
 * ENAMETOOLONG, EFAULT on the way; their order. */
#include "a_sys.h"

#define D "/tmp/lb_mkdir_errors"

static i64 mkdir_(const char *p, i64 mode) { return syscall2(SYS_mkdir, p, mode); }

static char buf[8192];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", mkdir_(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_sub", mkdir_(D "/sub", 0755), 0);
    expect("setup_link_dir", syscall2(SYS_symlink, "sub", D "/ldir"), 0);
    expect("setup_link_file", syscall2(SYS_symlink, "f", D "/lfile"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "target_dir", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);

    expect("exists_dir", mkdir_(D "/sub", 0755), -EEXIST);
    expect("exists_dir_slash", mkdir_(D "/sub/", 0755), -EEXIST);
    expect("exists_file", mkdir_(D "/f", 0755), -EEXIST);
    expect("exists_file_slash", mkdir_(D "/f/", 0755), -EEXIST);
    expect("exists_link_dir", mkdir_(D "/ldir", 0755), -EEXIST);
    expect("exists_link_file", mkdir_(D "/lfile", 0755), -EEXIST);
    expect("exists_dangling", mkdir_(D "/dangling", 0755), -EEXIST);
    expect("dangling_not_followed", a_stat_path(D "/target_dir", &st), -ENOENT);
    expect("exists_dangling_slash", mkdir_(D "/dangling/", 0755), -EEXIST);
    expect("exists_loop", mkdir_(D "/loop", 0755), -EEXIST);
    expect("exists_dot", mkdir_(D "/sub/.", 0755), -EEXIST);
    expect("exists_dotdot", mkdir_(D "/sub/..", 0755), -EEXIST);
    expect("exists_root", mkdir_("/", 0755), -EEXIST);
    expect("exists_dot_relative", mkdir_(".", 0755), -EEXIST);

    expect("missing_parent", mkdir_(D "/nope/x", 0755), -ENOENT);
    expect("missing_parent_slash", mkdir_(D "/nope/x/", 0755), -ENOENT);
    expect("empty", mkdir_("", 0755), -ENOENT);
    expect("dangling_parent", mkdir_(D "/dangling/x", 0755), -ENOENT);
    expect("file_parent", mkdir_(D "/f/x", 0755), -ENOTDIR);
    expect("linkfile_parent", mkdir_(D "/lfile/x", 0755), -ENOTDIR);
    expect("loop_parent", mkdir_(D "/loop/x", 0755), -ELOOP);
    expect("through_link_dir_ok", mkdir_(D "/ldir/new", 0755), 0);
    expect("made_in_target", a_stat_path(D "/sub/new", &st), 0);
    expect("null_efault", mkdir_((const char *)0, 0755), -EFAULT);
    expect("addr1_efault", mkdir_((const char *)1, 0755), -EFAULT);

    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 255; i++) p[i] = 'm';
    p[255] = 0;
    expect("name_255_ok", mkdir_(buf, 0755), 0);
    expect("name_255_rmdir", a_rmdir(buf), 0);
    p[255] = 'm';
    p[256] = 0;
    expect("name_256_enametoolong", mkdir_(buf, 0755), -ENAMETOOLONG);
    for (int i = 0; i < 4096; i++) buf[i] = '/';
    buf[4096] = 0;
    expect("path_4096_enametoolong", mkdir_(buf, 0755), -ENAMETOOLONG);
    buf[4095] = 0;
    expect("path_4095_root_eexist", mkdir_(buf, 0755), -EEXIST);
    /* order: missing parent before a long last name; an existing long name
     * cannot exist; ENOTDIR before ENAMETOOLONG */
    p = str_cpy(buf, D "/nope/");
    for (int i = 0; i < 256; i++) p[i] = 'm';
    p[256] = 0;
    expect("order_missing_before_long", mkdir_(buf, 0755), -ENOENT);
    p = str_cpy(buf, D "/f/");
    for (int i = 0; i < 256; i++) p[i] = 'm';
    p[256] = 0;
    expect("order_notdir_before_long", mkdir_(buf, 0755), -ENOTDIR);
    p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'm';
    str_cpy(p + 256, "/x");
    expect("order_long_parent", mkdir_(buf, 0755), -ENAMETOOLONG);

    expect("cleanup_new", a_rmdir(D "/sub/new"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_lfile", a_unlink(D "/lfile"), 0);
    expect("cleanup_ldir", a_unlink(D "/ldir"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
