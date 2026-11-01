/* truncate(2) errors: EINVAL for a negative length and for anything that
 * is not a regular file or directory (FIFO, device), EISDIR for a
 * directory, ENOENT (also a dangling link), ENOTDIR, ELOOP, ENAMETOOLONG,
 * EFAULT; their order. Failed calls leave the size alone. */
#include "a_sys.h"

#define D "/tmp/lb_truncate_errors"
#define SYS_mknodat 259

static i64 truncate_(const char *p, i64 len) { return syscall2(SYS_truncate, p, len); }

static char buf[8192];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "abc", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0666, 0), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nothing", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("setup_dl", syscall2(SYS_symlink, "sub", D "/dl"), 0);

    expect("negative", truncate_(D "/f", -1), -EINVAL);
    expect("negative_min", truncate_(D "/f", (i64)0x8000000000000000UL), -EINVAL);
    expect("size_kept", a_stat_path(D "/f", &st) == 0 && st.st_size == 3, 1);
    expect("dir_eisdir", truncate_(D "/sub", 0), -EISDIR);
    expect("dirlink_eisdir", truncate_(D "/dl", 0), -EISDIR);
    expect("fifo_einval", truncate_(D "/fifo", 0), -EINVAL);
    expect("devnull_einval", truncate_("/dev/null", 0), -EINVAL);
    expect("dangling_enoent", truncate_(D "/dangling", 0), -ENOENT);
    expect("loop_eloop", truncate_(D "/loop", 0), -ELOOP);
    expect("missing", truncate_(D "/nope", 0), -ENOENT);
    expect("missing_dir", truncate_(D "/nope/x", 0), -ENOENT);
    expect("empty", truncate_("", 0), -ENOENT);
    expect("file_as_dir", truncate_(D "/f/x", 0), -ENOTDIR);
    expect("file_slash", truncate_(D "/f/", 0), -ENOTDIR);
    expect("null_path", truncate_((const char *)0, 0), -EFAULT);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 't';
    p[256] = 0;
    expect("name_256", truncate_(buf, 0), -ENAMETOOLONG);
    for (int i = 0; i < 4096; i++) buf[i] = '/';
    buf[4096] = 0;
    expect("path_4096", truncate_(buf, 0), -ENAMETOOLONG);
    /* order: a negative length is reported before the path is looked at */
    expect("negative_vs_missing", truncate_(D "/nope", -1), -EINVAL);
    expect("negative_vs_null", truncate_((const char *)0, -1), -EINVAL);
    expect("negative_vs_dir", truncate_(D "/sub", -1), -EINVAL);
    p = str_cpy(buf, D "/nope/");
    for (int i = 0; i < 256; i++) p[i] = 't';
    p[256] = 0;
    expect("missing_before_long", truncate_(buf, 0), -ENOENT);
    expect("size_still_kept", a_stat_path(D "/f", &st) == 0 && st.st_size == 3, 1);

    static const char *rm[] = {D "/dl", D "/loop", D "/dangling", D "/fifo", D "/f"};
    for (unsigned i = 0; i < sizeof rm / sizeof rm[0]; i++) expect("cleanup_unlink", a_unlink(rm[i]), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
