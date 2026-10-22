/* chown path errors: a missing name or an empty path is ENOENT; a file used
 * as a directory (also "f/" and "f/.") is ENOTDIR; an unreadable path
 * pointer is EFAULT; a name of 256 bytes or a path of 4096 bytes with its
 * NUL is ENAMETOOLONG, while 255 bytes and 4095 bytes are not. */
#include "a_sys.h"

#define D "/tmp/c_chown_path_errors"

static i64 ch(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

static char name[300], path[5000];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("missing_enoent", ch(D "/nope", -1, -1), -ENOENT);
    expect("missing_dir_enoent", ch(D "/nope/f", -1, -1), -ENOENT);
    expect("empty_enoent", ch("", -1, -1), -ENOENT);
    expect("null_efault", ch((const char *)0, -1, -1), -EFAULT);
    expect("low_address_efault", ch((const char *)1, -1, -1), -EFAULT);
    expect("noncanonical_efault", ch((const char *)0x800000000000UL, -1, -1), -EFAULT);
    expect("file_as_dir_enotdir", ch(D "/f/x", -1, -1), -ENOTDIR);
    expect("file_trailing_slash_enotdir", ch(D "/f/", -1, -1), -ENOTDIR);
    expect("file_dot_enotdir", ch(D "/f/.", -1, -1), -ENOTDIR);
    for (int i = 0; i < 256; i++) name[i] = 'n';
    name[256] = 0;
    str_cat2(path, D "/", name);
    expect("name_256_enametoolong", ch(path, -1, -1), -ENAMETOOLONG);
    str_cat2(path, D "/f/", name);
    expect("name_256_after_file_enotdir", ch(path, -1, -1), -ENOTDIR);
    str_cat2(path, D "/nope/", name);
    expect("name_256_after_missing_enoent", ch(path, -1, -1), -ENOENT);
    name[255] = 0;
    str_cat2(path, D "/", name);
    expect("name_255_missing_enoent", ch(path, -1, -1), -ENOENT);
    expect("name_255_create", a_make_file(path, "", 0644), 0);
    expect("name_255_ok", ch(path, -1, -1), 0);
    expect("name_255_cleanup", a_unlink(path), 0);
    for (int i = 0; i < 4096; i++) path[i] = '/';
    path[4096] = 0;
    expect("path_4096_enametoolong", ch(path, -1, -1), -ENAMETOOLONG);
    path[4095] = 0;
    expect("path_4095_ok", ch(path, -1, -1), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
