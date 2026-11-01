/* utimes(2) errors: EINVAL for a tv_usec outside 0..999999 (checked
 * before the path), EFAULT for an unreadable times pointer, the usual path
 * errors; a failed call changes nothing. */
#include "a_sys.h"

#define D "/tmp/lb_utimes_errors"

static i64 utimes_(const char *p, const void *t) { return syscall2(SYS_utimes, p, t); }

static char buf[8192];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    struct a_timeval good[2] = {{100, 0}, {200, 0}};
    expect("set_known", utimes_(D "/f", good), 0);

    struct a_timeval t[2];
    t[0] = (struct a_timeval){1, 1000000};
    t[1] = (struct a_timeval){1, 0};
    expect("atime_usec_1e6", utimes_(D "/f", t), -EINVAL);
    t[0] = (struct a_timeval){1, 0};
    t[1] = (struct a_timeval){1, 1000000};
    expect("mtime_usec_1e6", utimes_(D "/f", t), -EINVAL);
    t[1] = (struct a_timeval){1, -1};
    expect("mtime_usec_neg", utimes_(D "/f", t), -EINVAL);
    t[0] = (struct a_timeval){1, -1};
    t[1] = (struct a_timeval){1, 0};
    expect("atime_usec_neg", utimes_(D "/f", t), -EINVAL);
    t[0] = (struct a_timeval){1, 999999};
    t[1] = (struct a_timeval){1, 999999};
    expect("usec_999999_ok", utimes_(D "/f", t), 0);
    expect("set_known_again", utimes_(D "/f", good), 0);
    /* UTIME_NOW / UTIME_OMIT have no meaning here (they are not whole
     * microseconds): out of range */
    t[0] = (struct a_timeval){0, (1L << 30) - 1};
    t[1] = (struct a_timeval){0, 0};
    expect("utime_now_value_einval", utimes_(D "/f", t), -EINVAL);
    expect("unchanged", a_stat_path(D "/f", &st) == 0 && st.st_atime_sec == 100 && st.st_mtime_sec == 200, 1);
    /* order: EINVAL before the path */
    t[0] = (struct a_timeval){1, 1000000};
    expect("einval_vs_missing", utimes_(D "/nope", t), -EINVAL);
    expect("einval_vs_null_path", utimes_((const char *)0, t), -EINVAL);
    expect("efault_times_vs_missing", utimes_(D "/nope", (void *)1), -EFAULT);
    expect("bad_times", utimes_(D "/f", (void *)1), -EFAULT);

    /* path errors */
    expect("setup_dangling", syscall2(SYS_symlink, "nothing", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("missing", utimes_(D "/nope", good), -ENOENT);
    expect("empty", utimes_("", good), -ENOENT);
    expect("dangling", utimes_(D "/dangling", good), -ENOENT);
    expect("loop", utimes_(D "/loop", good), -ELOOP);
    expect("file_as_dir", utimes_(D "/f/x", good), -ENOTDIR);
    expect("null_path", utimes_((const char *)0, good), -EFAULT);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'v';
    p[256] = 0;
    expect("name_256", utimes_(buf, good), -ENAMETOOLONG);
    expect("unchanged_end", a_stat_path(D "/f", &st) == 0 && st.st_atime_sec == 100 && st.st_mtime_sec == 200, 1);

    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
