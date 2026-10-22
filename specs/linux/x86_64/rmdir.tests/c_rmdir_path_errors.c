/* rmdir path errors and their order. A missing last name or a missing
 * directory on the way is ENOENT (also when a later ".." would leave
 * it), an empty path ENOENT, a non-directory on the way ENOTDIR, a
 * symbolic link loop on the way ELOOP. Where several apply, the first
 * one met walking from the left decides; a problem with the last name
 * itself ("." EINVAL, ".." ENOTEMPTY) comes after all problems on the
 * way. */
#include "a_sys.h"

#define D "/tmp/c_rmdir_path_errors"

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }
static i64 sl(const char *target, const char *path) { return syscall3(SYS_symlinkat, target, AT_FDCWD, path); }

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_loop_a", sl("lb", D "/la"), 0);
    expect("setup_loop_b", sl("la", D "/lb"), 0);
    expect("setup_dangling", sl("nowhere", D "/dang"), 0);

    expect("missing_last", rm(D "/nope"), -ENOENT);
    expect("missing_last_slash", rm(D "/nope/"), -ENOENT);
    expect("missing_on_way", rm(D "/nope/x"), -ENOENT);
    expect("missing_on_way_dotdot", rm(D "/nope/../d"), -ENOENT);
    expect("d_kept_after_dotdot", a_stat_path(D "/d", &st), 0);
    expect("empty_path", rm(""), -ENOENT);
    expect("file_on_way", rm(D "/f/x"), -ENOTDIR);
    expect("file_on_way_deeper", rm(D "/f/x/y"), -ENOTDIR);
    expect("file_dot", rm(D "/f/."), -ENOTDIR);
    expect("file_dotdot", rm(D "/f/.."), -ENOTDIR);
    expect("file_dotdot_d", rm(D "/f/../d"), -ENOTDIR);
    expect("loop_on_way", rm(D "/la/x"), -ELOOP);
    expect("loop_on_way_dot", rm(D "/la/."), -ELOOP);
    expect("dangling_on_way", rm(D "/dang/x"), -ENOENT);
    expect("dangling_dot", rm(D "/dang/."), -ENOENT);

    /* Order: the first problem met from the left. */
    expect("missing_before_file", rm(D "/nope/f/x"), -ENOENT);
    expect("file_before_missing", rm(D "/f/nope/x"), -ENOTDIR);
    expect("missing_before_loop", rm(D "/nope/la/x"), -ENOENT);
    expect("loop_before_missing", rm(D "/la/nope/x"), -ELOOP);
    expect("missing_before_dot", rm(D "/nope/."), -ENOENT);
    expect("missing_before_dotdot", rm(D "/nope/.."), -ENOENT);
    expect("d_dot_einval", rm(D "/d/."), -EINVAL);

    /* Relative paths start at the working directory. */
    expect("chdir_D", syscall1(SYS_chdir, D), 0);
    expect("rel_missing", rm("nope"), -ENOENT);
    expect("rel_file", rm("f"), -ENOTDIR);
    expect("rel_file_on_way", rm("f/x"), -ENOTDIR);
    expect("rel_rmdir_d", rm("d"), 0);
    expect("rel_d_gone", a_lstat_path(D "/d", &st), -ENOENT);
    expect("rel_mk_d", a_mkdir(D "/d", 0755), 0);
    expect("rel_dot_slash", rm("./d"), 0);
    expect("rel_mk_d2", a_mkdir(D "/d", 0755), 0);
    expect("rel_up_and_back", rm("../c_rmdir_path_errors/d"), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("abs_still_works_rel_gone", rm("f"), -ENOENT);

    expect("cleanup", a_unlink(D "/f") | a_unlink(D "/la") | a_unlink(D "/lb") | a_unlink(D "/dang"), 0);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
