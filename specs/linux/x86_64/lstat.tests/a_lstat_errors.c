/* lstat: path errors. ENOENT for a missing name or an empty path, ENOTDIR
 * for a file used as a directory, ENAMETOOLONG for a 256-byte name or a
 * 4096-byte path, EFAULT for an unreadable path. */
#include "a_sys.h"

#define D "/tmp/a_lstat_errors"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }

static char buf[8192];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0600), 0);
    expect("missing_enoent", lstat(D "/nope", &st), -ENOENT);
    expect("missing_parent_enoent", lstat(D "/nope/x", &st), -ENOENT);
    expect("empty_enoent", lstat("", &st), -ENOENT);
    expect("file_as_dir_enotdir", lstat(D "/f/x", &st), -ENOTDIR);
    expect("file_trailing_slash_enotdir", lstat(D "/f/", &st), -ENOTDIR);
    expect("file_trailing_dot_enotdir", lstat(D "/f/.", &st), -ENOTDIR);
    /* A 255-byte name is fine (missing), 256 bytes is too long. */
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 255; i++) p[i] = 'a';
    p[255] = 0;
    expect("name_255_enoent", lstat(buf, &st), -ENOENT);
    p[255] = 'a';
    p[256] = 0;
    expect("name_256_enametoolong", lstat(buf, &st), -ENAMETOOLONG);
    /* A long path of short components: 4095 bytes + NUL is allowed,
     * 4096 + NUL is too long. */
    for (int i = 0; i < 4095; i++) buf[i] = (i % 2) ? '/' : '.';
    buf[0] = '/';
    buf[4095] = 0;
    expect("path_4095_ok", lstat(buf, &st), 0);
    expect("path_4095_is_root", st.st_mode & S_IFMT, S_IFDIR);
    buf[4095] = '/';
    buf[4096] = 0;
    expect("path_4096_enametoolong", lstat(buf, &st), -ENAMETOOLONG);
    expect("null_path_efault", lstat((const char *)0, &st), -EFAULT);
    expect("bad_path_efault", lstat((const char *)1, &st), -EFAULT);
    expect("kernel_path_efault", lstat((const char *)0xffffffffff600000ul, &st), -EFAULT);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
