/* mkdir and lengths: a last name of 255 bytes works, 256 is
 * ENAMETOOLONG; a whole path of 4095 bytes (4096 with its NUL) works,
 * one byte more is ENAMETOOLONG even when every name in it is short.
 * Order: the whole-path length comes first (a 4096-byte path under a
 * missing directory is ENAMETOOLONG); otherwise the walk decides from
 * left to right, so a missing directory before a long name is ENOENT, a
 * file before it ENOTDIR, a long name before a missing one ENAMETOOLONG. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_name_length"

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static char buf[8192];

/* buf = prefix followed by n copies of c, then suffix. */
static char *build(const char *prefix, char c, size_t n, const char *suffix) {
    str_cpy(buf, prefix);
    size_t l = str_len(buf);
    memset(buf + l, c, n);
    str_cpy(buf + l + n, suffix);
    return buf;
}

/* A path of exactly len bytes: D/ then "./" pairs, then '/'s, then "z". */
static char *dots_path(size_t len) {
    str_cpy(buf, D "/");
    size_t l = str_len(buf);
    while (l + 2 < len) {
        buf[l++] = '.';
        buf[l++] = '/';
    }
    while (l + 1 < len) buf[l++] = '/';
    buf[l++] = 'z';
    buf[l] = 0;
    return buf;
}

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);

    expect("name_255", mk(build(D "/", 'n', 255, ""), 0700), 0);
    expect("name_255_exists", a_lstat_path(buf, &st), 0);
    expect("name_255_rmdir", a_rmdir(buf), 0);
    expect("name_256", mk(build(D "/", 'n', 256, ""), 0700), -ENAMETOOLONG);
    expect("name_256_slash", mk(build(D "/", 'n', 256, "/"), 0700), -ENAMETOOLONG);
    expect("name_1000", mk(build(D "/", 'n', 1000, ""), 0700), -ENAMETOOLONG);

    expect("path_4095", mk(dots_path(4095), 0700), 0);
    expect("path_4095_len", str_len(buf), 4095);
    expect("z_made", a_lstat_path(D "/z", &st), 0);
    expect("rm_z", a_rmdir(D "/z"), 0);
    expect("path_4096", mk(dots_path(4096), 0700), -ENAMETOOLONG);
    expect("z_not_made", a_lstat_path(D "/z", &st), -ENOENT);
    expect("path_4096_missing_dir", mk(build(D "/no/", '/', 4096 - str_len(D "/no/") - 1, "z"), 0700),
           -ENAMETOOLONG);
    expect("path_4096_len", str_len(buf), 4096);
    expect("path_4095_missing_dir", mk(build(D "/no/", '/', 4095 - str_len(D "/no/") - 1, "z"), 0700), -ENOENT);

    expect("missing_then_long", mk(build(D "/no/", 'n', 256, ""), 0700), -ENOENT);
    expect("missing_then_long_then_more", mk(build(D "/no/", 'n', 256, "/x"), 0700), -ENOENT);
    expect("missing_dotdot_long", mk(build(D "/no/../", 'n', 256, ""), 0700), -ENOENT);
    expect("file_then_long", mk(build(D "/f/", 'n', 256, ""), 0700), -ENOTDIR);
    expect("long_then_missing", mk(build(D "/", 'n', 256, "/no/x"), 0700), -ENAMETOOLONG);
    expect("long_dir_on_way", mk(build(D "/", 'n', 256, "/x"), 0700), -ENAMETOOLONG);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
