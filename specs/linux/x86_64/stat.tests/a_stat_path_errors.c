/* stat: errors from the path: missing names, empty paths, non-directories
 * used as directories, over-long names and paths. */
#include "a_sys.h"

#define D "/tmp/a_stat_path_errors"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }

static char buf[8192];

/* `prefix` followed by `n` copies of `c`, then `suffix`. */
static char *fill(const char *prefix, char c, int n, const char *suffix) {
    char *p = str_cpy(buf, prefix);
    for (int i = 0; i < n; i++) *p++ = c;
    str_cpy(p, suffix);
    return buf;
}

int test_main(void) {
    struct a_stat s;
    a_unlink(D "/f");
    a_unlink(fill(D "/", 'n', 255, ""));
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);

    expect("missing_enoent", st(D "/nope", &s), -ENOENT);
    expect("missing_dir_enoent", st(D "/nope/x", &s), -ENOENT);
    expect("missing_top_enoent", st("/a_stat_no_such_dir/x", &s), -ENOENT);
    expect("empty_path_enoent", st("", &s), -ENOENT);
    expect("file_as_dir_enotdir", st(D "/f/x", &s), -ENOTDIR);
    expect("file_trailing_slash_enotdir", st(D "/f/", &s), -ENOTDIR);
    expect("file_trailing_dot_enotdir", st(D "/f/.", &s), -ENOTDIR);
    expect("file_dotdot_enotdir", st(D "/f/..", &s), -ENOTDIR);
    expect("devnull_as_dir_enotdir", st("/dev/null/x", &s), -ENOTDIR);

    /* Names: 255 bytes is the limit for one component. */
    expect("name_255_create", a_make_file(fill(D "/", 'n', 255, ""), "", 0644), 0);
    expect("name_255_ok", st(fill(D "/", 'n', 255, ""), &s), 0);
    expect("name_256_enametoolong", st(fill(D "/", 'n', 256, ""), &s), -ENAMETOOLONG);
    expect("name_256_missing_dir_first", st(fill(D "/nope/", 'n', 256, ""), &s), -ENOENT);
    expect("name_256_after_file_enotdir", st(fill(D "/f/", 'n', 256, ""), &s), -ENOTDIR);
    expect("name_256_then_more_enametoolong", st(fill(D "/", 'n', 256, "/nope/x"), &s), -ENAMETOOLONG);
    expect("missing_then_dotdot_enoent", st(D "/nope/../f", &s), -ENOENT);

    /* Paths: 4095 bytes plus the NUL is the most; components of "./"
     * pairs keep every name short. */
    char *p = str_cpy(buf, D);
    while (p - buf < 4095 - 2) *p++ = '/';
    str_cpy(p, "/f");
    expect("path_4095_len", (i64)str_len(buf), 4095);
    expect("path_4095_ok", st(buf, &s), 0);
    p = str_cpy(buf, D);
    while (p - buf < 4096 - 2) *p++ = '/';
    str_cpy(p, "/f");
    expect("path_4096_len", (i64)str_len(buf), 4096);
    expect("path_4096_enametoolong", st(buf, &s), -ENAMETOOLONG);
    expect("path_6000_missing_enametoolong", st(fill("/nope/", 'n', 6000, ""), &s), -ENAMETOOLONG);

    p = buf;
    *p++ = '/';
    while (p - buf < 4094 - (i64)sizeof(D "/f")) {
        *p++ = '.';
        *p++ = '/';
    }
    str_cpy(p, D + 1);
    str_cpy(buf + str_len(buf), "/f");
    check("dot_pairs_len_under_4096", str_len(buf) < 4096 && str_len(buf) > 4000, (i64)str_len(buf));
    expect("dot_pairs_ok", st(buf, &s), 0);
    expect("dot_pairs_is_file", s.st_mode & S_IFMT, S_IFREG);

    a_unlink(fill(D "/", 'n', 255, ""));
    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
