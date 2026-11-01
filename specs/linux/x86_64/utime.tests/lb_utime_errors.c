/* utime(2) errors: path errors (ENOENT also for a dangling link, ENOTDIR,
 * ELOOP, ENAMETOOLONG, EFAULT), EFAULT for an unreadable times pointer
 * (after the path), EACCES for a closed directory on the way; an owner
 * may set any times on its own file whatever its mode. */
#include "a_sys.h"

#define D "/tmp/lb_utime_errors"
#define SYS_utime 132

struct utimbuf {
    i64 actime, modtime;
};

static i64 utime_(const char *p, const void *t) { return syscall2(SYS_utime, p, t); }

static char buf[8192];

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    struct a_stat st;
    struct utimbuf t = {100, 200};
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_g", a_make_file(D "/sub/g", "x", 0644), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nothing", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);

    expect("missing", utime_(D "/nope", &t), -ENOENT);
    expect("missing_dir", utime_(D "/nope/x", &t), -ENOENT);
    expect("empty", utime_("", &t), -ENOENT);
    expect("dangling", utime_(D "/dangling", &t), -ENOENT);
    expect("loop", utime_(D "/loop", &t), -ELOOP);
    expect("file_as_dir", utime_(D "/f/x", &t), -ENOTDIR);
    expect("file_slash", utime_(D "/f/", &t), -ENOTDIR);
    expect("null_path", utime_((const char *)0, &t), -EFAULT);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'u';
    p[256] = 0;
    expect("name_256", utime_(buf, &t), -ENAMETOOLONG);
    for (int i = 0; i < 4096; i++) buf[i] = '/';
    buf[4096] = 0;
    expect("path_4096", utime_(buf, &t), -ENAMETOOLONG);

    /* the times pointer */
    expect("bad_times", utime_(D "/f", (void *)1), -EFAULT);
    void *two = (void *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, (char *)two + A_PAGE, A_PAGE);
    expect("times_runs_off_page", utime_(D "/f", (char *)two + A_PAGE - 8), -EFAULT);
    struct utimbuf *edge = (struct utimbuf *)((char *)two + A_PAGE - 16);
    edge->actime = 7;
    edge->modtime = 8;
    expect("times_at_page_end_ok", utime_(D "/f", edge), 0);
    expect("edge_values", a_stat_path(D "/f", &st) == 0 && st.st_atime_sec == 7 && st.st_mtime_sec == 8, 1);
    /* order: the times are read before the path is looked up */
    expect("bad_times_missing_path", utime_(D "/nope", (void *)1), -EFAULT);
    expect("bad_times_null_path", utime_((const char *)0, (void *)1), -EFAULT);
    expect("unchanged_after_efault", a_stat_path(D "/f", &st) == 0 && st.st_mtime_sec == 8, 1);

    /* the owner may set explicit times or "now" whatever the mode */
    expect("chmod_0", syscall2(SYS_chmod, D "/f", 0), 0);
    expect("owner_mode0_explicit", utime_(D "/f", &t), 0);
    expect("owner_mode0_now", utime_(D "/f", 0), 0);
    expect("chmod_back", syscall2(SYS_chmod, D "/f", 0644), 0);
    /* search permission */
    expect("chmod_sub_0600", syscall2(SYS_chmod, D "/sub", 0600), 0);
    expect("closed_dir", utime_(D "/sub/g", &t), root ? 0 : -EACCES);
    expect("chmod_sub_back", syscall2(SYS_chmod, D "/sub", 0755), 0);

    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
