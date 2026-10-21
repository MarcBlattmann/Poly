/* open(2): errors from the path string and its components: ENOENT,
 * ENOTDIR, EISDIR, ENAMETOOLONG, EFAULT, and which wins when several
 * apply. */
#include "a_sys.h"

#define SYS_open 2
#define D "/tmp/a_open_path_errors"

static i64 op(const char *p, i64 flags, i64 mode) { return syscall3(SYS_open, p, flags, mode); }

static char big[5000];

/* D "/" followed by `n` copies of 'a' then `tail`. */
static const char *longname(int n, const char *tail) {
    char *p = str_cpy(big, D "/");
    for (int i = 0; i < n; i++) *p++ = 'a';
    str_cpy(p, tail);
    return big;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0600), 0);

    expect("empty_path_enoent", op("", O_RDONLY, 0), -ENOENT);
    expect("empty_path_creat_enoent", op("", O_RDWR | O_CREAT, 0600), -ENOENT);
    expect("null_path_efault", op(0, O_RDONLY, 0), -EFAULT);
    expect("bad_pointer_efault", op((const char *)0x10, O_RDONLY, 0), -EFAULT);
    expect("kernel_pointer_efault", op((const char *)0xffff800000000000UL, O_RDONLY, 0), -EFAULT);

    expect("missing_enoent", op(D "/nope", O_RDONLY, 0), -ENOENT);
    expect("missing_dir_enoent", op(D "/nope/x", O_RDONLY, 0), -ENOENT);
    expect("missing_dir_creat_enoent", op(D "/nope/x", O_RDWR | O_CREAT, 0600), -ENOENT);
    expect("file_as_dir_enotdir", op(D "/f/x", O_RDONLY, 0), -ENOTDIR);
    expect("file_as_dir_creat_enotdir", op(D "/f/x", O_RDWR | O_CREAT, 0600), -ENOTDIR);
    expect("file_trailing_slash_enotdir", op(D "/f/", O_RDONLY, 0), -ENOTDIR);
    expect("file_trailing_dot_enotdir", op(D "/f/.", O_RDONLY, 0), -ENOTDIR);
    expect("file_trailing_dotdot_enotdir", op(D "/f/..", O_RDONLY, 0), -ENOTDIR);
    expect("file_trailing_slash_creat_eisdir", op(D "/f/", O_RDWR | O_CREAT, 0600), -EISDIR);
    expect("new_trailing_slash_creat_eisdir", op(D "/new/", O_RDWR | O_CREAT, 0600), -EISDIR);
    expect("new_trailing_slash_creat_not_created", a_stat_path(D "/new", (struct a_stat *)big), -ENOENT);
    expect("missing_trailing_slash_enoent", op(D "/new/", O_RDONLY, 0), -ENOENT);

    /* Slashes and dots. */
    i64 fd = op("//tmp//a_open_path_errors///f", O_RDONLY, 0);
    expect("repeated_slashes_ok", fd, 3);
    a_close(fd);
    fd = op(D "/./././f", O_RDONLY, 0);
    expect("dot_components_ok", fd, 3);
    a_close(fd);
    fd = op("/../../tmp/../tmp/a_open_path_errors/f", O_RDONLY, 0);
    expect("dotdot_above_root_ok", fd, 3);
    a_close(fd);
    expect("missing_then_dotdot_enoent", op(D "/nope/../f", O_RDONLY, 0), -ENOENT);

    /* Name length: 255 bytes is fine, 256 is ENAMETOOLONG. */
    fd = op(longname(255, ""), O_RDWR | O_CREAT, 0600);
    expect("name_255_creates", fd, 3);
    a_close(fd);
    expect("name_255_unlink", a_unlink(longname(255, "")), 0);
    expect("name_256_enametoolong", op(longname(256, ""), O_RDWR | O_CREAT, 0600), -ENAMETOOLONG);
    expect("name_256_rdonly_enametoolong", op(longname(256, ""), O_RDONLY, 0), -ENAMETOOLONG);
    expect("name_256_then_missing_enametoolong", op(longname(256, "/x"), O_RDONLY, 0), -ENAMETOOLONG);

    /* A missing directory before an over-long name decides. */
    char *p = str_cpy(big, D "/nope/");
    for (int i = 0; i < 300; i++) *p++ = 'b';
    *p = 0;
    expect("missing_before_long_name_enoent", op(big, O_RDONLY, 0), -ENOENT);

    /* Whole path: 4095 bytes plus NUL is accepted by the length check,
     * 4096 plus NUL is not, whatever the components are. */
    p = str_cpy(big, "/");
    while (p - big < 4095) *p++ = '/';
    *p = 0;
    fd = op(big, O_RDONLY, 0);
    expect("path_4095_slashes_ok", fd, 3);
    a_close(fd);
    *p++ = '/';
    *p = 0;
    expect("path_4096_enametoolong", op(big, O_RDONLY, 0), -ENAMETOOLONG);
    p = str_cpy(big, D "/nope/");
    while (p - big < 4200) *p++ = 'c';
    *p = 0;
    expect("path_4200_missing_dir_enametoolong", op(big, O_RDONLY, 0), -ENAMETOOLONG);

    expect("cleanup_unlink", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
