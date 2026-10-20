/* openat: paths relative to a directory descriptor, bad descriptors,
 * empty, overlong and unreadable paths. */
#include "a_sys.h"

#define D "/tmp/a_openat_dirfd"

static char longpath[4200];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_subdir", a_mkdir(D "/sub", 0700), 0);
    expect("setup_file", a_make_file(D "/sub/f", "x", 0644), 0);

    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("open_dir", dfd, 3);
    i64 fd = syscall4(SYS_openat, dfd, "sub/f", O_RDONLY, 0);
    expect("relative_to_dirfd", fd, 4);
    a_close(fd);
    fd = syscall4(SYS_openat, dfd, "sub/../sub/./f", O_RDONLY, 0);
    expect("dot_and_dotdot_components", fd, 4);
    a_close(fd);
    fd = syscall4(SYS_openat, dfd, "new", O_RDWR | O_CREAT | O_EXCL, 0600);
    expect("create_relative_to_dirfd", fd, 4);
    a_close(fd);
    expect("created_where_dirfd_points", a_unlink(D "/new"), 0);

    i64 ffd = a_open(D "/sub/f", O_RDONLY, 0);
    expect("open_file", ffd, 4);
    expect("dirfd_is_file_enotdir", syscall4(SYS_openat, ffd, "x", O_RDONLY, 0), -ENOTDIR);
    expect("bad_dirfd_ebadf", syscall4(SYS_openat, 99, "x", O_RDONLY, 0), -EBADF);
    expect("negative_dirfd_ebadf", syscall4(SYS_openat, -5, "x", O_RDONLY, 0), -EBADF);
    fd = syscall4(SYS_openat, 99, D "/sub/f", O_RDONLY, 0);
    expect("absolute_path_ignores_bad_dirfd", fd, 5);
    a_close(fd);
    a_close(ffd);

    expect("empty_path_enoent", a_open("", O_RDONLY, 0), -ENOENT);
    expect("empty_path_with_dirfd_enoent", syscall4(SYS_openat, dfd, "", O_RDONLY, 0), -ENOENT);
    expect("null_path_efault", a_open((const char *)0, O_RDONLY, 0), -EFAULT);
    expect("bad_path_pointer_efault", a_open((const char *)8, O_RDONLY, 0), -EFAULT);

    /* A component longer than NAME_MAX (255). */
    char *p = str_cpy(longpath, D "/");
    for (int i = 0; i < 256; i++) *p++ = 'a';
    *p = 0;
    expect("long_component_enametoolong", a_open(longpath, O_RDONLY, 0), -ENAMETOOLONG);
    /* A path of PATH_MAX (4096) bytes or more, counting the NUL. */
    p = longpath;
    for (int i = 0; i < 2048; i++) *p++ = '/', *p++ = '.';
    *p = 0;
    expect("path_4096_bytes_enametoolong", a_open(longpath, O_RDONLY, 0), -ENAMETOOLONG);
    longpath[4094] = 0; /* 4094 characters + NUL: within PATH_MAX */
    fd = a_open(longpath, O_RDONLY, 0);
    expect("path_4095_bytes_opens_root", fd, 4);
    a_close(fd);

    a_close(dfd);
    expect("cleanup_unlink", a_unlink(D "/sub/f"), 0);
    expect("cleanup_rmdir_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
