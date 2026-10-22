/* truncate path errors: a missing file or directory and an empty path
 * are ENOENT (no file is created); a regular file used as a directory,
 * also with a trailing slash, is ENOTDIR; a name of 256 bytes or a path
 * of 4096 bytes or more (with its NUL) is ENAMETOOLONG, while 255 bytes
 * and 4095 bytes are only looked up (ENOENT here); an unreadable path
 * pointer is EFAULT. */
#include "a_sys.h"

#define D "/tmp/a_truncate_path_errors"

static char name[300], path[4200];

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0644), 0);
    expect("missing", syscall2(SYS_truncate, D "/none", 0), -ENOENT);
    expect("missing_not_created", a_stat_path(D "/none", &st), -ENOENT);
    expect("missing_grow", syscall2(SYS_truncate, D "/none", 10), -ENOENT);
    expect("missing_dir", syscall2(SYS_truncate, D "/none/f", 0), -ENOENT);
    expect("empty", syscall2(SYS_truncate, "", 0), -ENOENT);
    expect("file_as_dir", syscall2(SYS_truncate, D "/f/x", 0), -ENOTDIR);
    expect("file_trailing_slash", syscall2(SYS_truncate, D "/f/", 0), -ENOTDIR);
    expect("file_dot", syscall2(SYS_truncate, D "/f/.", 0), -ENOTDIR);
    expect("null", syscall2(SYS_truncate, 0, 0), -EFAULT);
    expect("bad_pointer", syscall2(SYS_truncate, 8, 0), -EFAULT);
    expect("kernel_pointer", syscall2(SYS_truncate, (i64)0xffff800000000000UL, 0), -EFAULT);
    for (int i = 0; i < 256; i++) name[i] = 'n';
    name[256] = 0;
    str_cat2(path, D "/", name);
    expect("name_256", syscall2(SYS_truncate, path, 0), -ENAMETOOLONG);
    name[255] = 0;
    str_cat2(path, D "/", name);
    expect("name_255", syscall2(SYS_truncate, path, 0), -ENOENT);
    /* "/ddd.../ddd" of 4095 bytes, components of 99 bytes. */
    int n = 0;
    while (n < 4095) {
        path[n] = (n % 100) ? 'd' : '/';
        n++;
    }
    path[n] = 0;
    expect("path_4095", syscall2(SYS_truncate, path, 0), -ENOENT);
    path[n] = 'd';
    path[n + 1] = 0;
    expect("path_4096", syscall2(SYS_truncate, path, 0), -ENAMETOOLONG);
    a_stat_path(D "/f", &st);
    expect("file_untouched", st.st_size, 3);
    /* A relative path is looked up from the working directory. */
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("relative", syscall2(SYS_truncate, "f", 1), 0);
    expect("relative_dot", syscall2(SYS_truncate, "./f", 2), 0);
    expect("relative_missing", syscall2(SYS_truncate, "none", 0), -ENOENT);
    a_stat_path(D "/f", &st);
    expect("relative_result", st.st_size, 2);
    expect("chdir_back", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
