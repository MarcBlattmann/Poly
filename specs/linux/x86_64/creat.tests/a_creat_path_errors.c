/* creat(2) path errors: missing directories, an empty path, a file used as
 * a directory, names that are too long, and bad pointers. Nothing is
 * created when the call fails. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_path_errors"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

static char big[4200];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);

    expect("missing_dir_enoent", cr(D "/no/x", 0644), -ENOENT);
    expect("missing_dir_not_created", a_stat_path(D "/no", &st), -ENOENT);
    expect("missing_then_dotdot_enoent", cr(D "/no/../x", 0644), -ENOENT);
    expect("dotdot_not_created", a_stat_path(D "/x", &st), -ENOENT);
    expect("empty_path_enoent", cr("", 0644), -ENOENT);

    expect("file_as_dir_enotdir", cr(D "/f/x", 0644), -ENOTDIR);
    expect("file_dot_enotdir", cr(D "/f/.", 0644), -ENOTDIR);
    expect("file_dotdot_enotdir", cr(D "/f/..", 0644), -ENOTDIR);
    expect("file_still_size_1", a_stat_path(D "/f", &st) == 0 ? (i64)st.st_size : -1, 1);

    /* A component of 255 bytes is fine, 256 is too long. */
    char name[300];
    int n = 0;
    for (const char *s = D "/"; *s; s++) name[n++] = *s;
    int base = n;
    for (int i = 0; i < 255; i++) name[n++] = 'a';
    name[n] = 0;
    i64 fd = cr(name, 0644);
    expect("name_255_ok", fd, 3);
    if (fd >= 0) a_close(fd);
    expect("name_255_unlink", a_unlink(name), 0);
    name[n++] = 'a';
    name[n] = 0;
    expect("name_256_enametoolong", cr(name, 0644), -ENAMETOOLONG);
    (void)base;

    /* A path of 4096 bytes with its NUL is too long even if made of
     * short components; 4095 plus NUL is looked up normally. */
    for (int i = 0; i < 4095; i++) big[i] = (i % 2) ? '/' : '.';
    big[0] = '/';
    big[4095] = 0;
    i64 r = cr(big, 0644);
    check("path_4095_not_enametoolong", r != -ENAMETOOLONG, r);
    if (r >= 0) a_close(r);
    big[4095] = '.';
    big[4096] = 0;
    expect("path_4096_enametoolong", cr(big, 0644), -ENAMETOOLONG);

    expect("null_efault", cr((const char *)0, 0644), -EFAULT);
    expect("low_addr_efault", cr((const char *)0x10, 0644), -EFAULT);
    expect("kernel_addr_efault", cr((const char *)0xffff800000000000ULL, 0644), -EFAULT);

    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
