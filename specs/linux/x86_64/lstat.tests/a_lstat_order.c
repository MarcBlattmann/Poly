/* lstat: when a path has more than one problem, the whole string is checked
 * first (EFAULT, ENAMETOOLONG for 4096 bytes), then the components left
 * to right: the first one that fails decides. */
#include "a_sys.h"

#define D "/tmp/a_lstat_order"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }

static char buf[8192];
static char lng[300];

static const char *path(const char *a, const char *b) {
    char *p = str_cpy(buf, a);
    str_cpy(p, b);
    return buf;
}

int test_main(void) {
    struct a_stat st;
    for (int i = 0; i < 256; i++) lng[i] = 'n';
    lng[256] = 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_loop", syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop"), 0);
    char tmp[400];
    str_cat2(tmp, "/nope/", lng);
    expect("missing_then_long_enoent", lstat(path(D, tmp), &st), -ENOENT);
    str_cat2(tmp, "/nope/../", lng);
    expect("missing_dotdot_long_enoent", lstat(path(D, tmp), &st), -ENOENT);
    str_cat2(tmp, "/f/", lng);
    expect("file_then_long_enotdir", lstat(path(D, tmp), &st), -ENOTDIR);
    str_cat2(tmp, "/loop/", lng);
    expect("loop_then_long_eloop", lstat(path(D, tmp), &st), -ELOOP);
    str_cat2(tmp, "/", lng);
    str_cat2(tmp + str_len(tmp), "/", "nope");
    expect("long_then_missing_enametoolong", lstat(path(D, tmp), &st), -ENAMETOOLONG);
    expect("long_last_enametoolong", lstat(path(D "/", lng), &st), -ENAMETOOLONG);
    /* A 4096-byte path is refused before any component is looked at. */
    char *p = str_cpy(buf, "/nope/");
    while (p - buf < 4096) *p++ = 'a';
    *p = 0;
    expect("whole_too_long_first", lstat(buf, &st), -ENAMETOOLONG);
    expect("efault_before_buf", lstat((const char *)8, (struct a_stat *)8), -EFAULT);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
