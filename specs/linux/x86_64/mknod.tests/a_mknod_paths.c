/* mknod: path errors. Existing names of any kind (also a dangling symbolic
 * link, ".", "..", "/") are EEXIST; a trailing slash on a new name is
 * ENOENT; missing or non-directory components, bad pointers, long names
 * and symbolic link loops fail as for other path calls. */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_paths"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

static char path[4200];

int test_main(void) {
    struct a_stat st;
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("setup_dir", a_mkdir(D "/d", 0755), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "nowhere", AT_FDCWD, D "/dl"), 0);
    expect("setup_loop", syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop"), 0);
    expect("setup_dirlink", syscall3(SYS_symlinkat, "d", AT_FDCWD, D "/dirlink"), 0);

    expect("over_file_eexist", mk(D "/f", S_IFIFO | 0644, 0), -EEXIST);
    expect("over_dir_eexist", mk(D "/d", S_IFIFO | 0644, 0), -EEXIST);
    expect("over_dangling_eexist", mk(D "/dl", S_IFIFO | 0644, 0), -EEXIST);
    expect("dangling_target_not_made", a_lstat_path(D "/nowhere", &st), -ENOENT);
    expect("over_loop_eexist", mk(D "/loop", S_IFIFO | 0644, 0), -EEXIST);
    expect("over_dirlink_eexist", mk(D "/dirlink", 0644, 0), -EEXIST);
    expect("dot_eexist", mk(D "/.", S_IFIFO | 0644, 0), -EEXIST);
    expect("dotdot_eexist", mk(D "/..", S_IFIFO | 0644, 0), -EEXIST);
    expect("root_eexist", mk("/", S_IFIFO | 0644, 0), -EEXIST);
    expect("dir_trailing_slash_eexist", mk(D "/d/", S_IFIFO | 0644, 0), -EEXIST);
    expect("file_trailing_slash_eexist", mk(D "/f/", S_IFIFO | 0644, 0), -EEXIST);
    expect("file_kept", a_lstat_path(D "/f", &st) == 0 ? st.st_size : -1, 4);

    expect("new_trailing_slash_enoent", mk(D "/n/", S_IFIFO | 0644, 0), -ENOENT);
    expect("new_trailing_slash_reg_enoent", mk(D "/n/", 0644, 0), -ENOENT);
    expect("new_trailing_slash_sock_enoent", mk(D "/n/", S_IFSOCK | 0644, 0), -ENOENT);
    expect("trailing_slash_not_made", a_lstat_path(D "/n", &st), -ENOENT);
    expect("missing_parent_enoent", mk(D "/nope/x", S_IFIFO | 0644, 0), -ENOENT);
    expect("under_dangling_enoent", mk(D "/dl/x", S_IFIFO | 0644, 0), -ENOENT);
    expect("empty_enoent", mk("", S_IFIFO | 0644, 0), -ENOENT);
    expect("under_file_enotdir", mk(D "/f/x", S_IFIFO | 0644, 0), -ENOTDIR);
    expect("null_efault", mk((const char *)0, S_IFIFO | 0644, 0), -EFAULT);
    expect("bad_pointer_efault", mk((const char *)8, S_IFIFO | 0644, 0), -EFAULT);
    expect("loop_eloop", mk(D "/loop/x", S_IFIFO | 0644, 0), -ELOOP);

    expect("through_dirlink", mk(D "/dirlink/v", S_IFIFO | 0644, 0), 0);
    expect("through_dirlink_made", a_lstat_path(D "/d/v", &st) == 0 ? st.st_mode : 0, S_IFIFO | 0644);
    expect("dotdot_inside", mk(D "/d/../w", 0644, 0), 0);
    expect("dotdot_inside_made", a_lstat_path(D "/w", &st), 0);
    expect("double_slashes", mk("/tmp//a_mknod_paths///s", S_IFSOCK | 0644, 0), 0);
    expect("double_slashes_made", a_lstat_path(D "/s", &st), 0);

    char *p = str_cpy(path, D "/");
    for (int i = 0; i < 255; i++) *p++ = 'n';
    *p = 0;
    expect("name_255", mk(path, S_IFIFO | 0644, 0), 0);
    expect("name_255_made", a_lstat_path(path, &st), 0);
    expect("name_255_unlink", a_unlink(path), 0);
    *p++ = 'n';
    *p = 0;
    expect("name_256_enametoolong", mk(path, S_IFIFO | 0644, 0), -ENAMETOOLONG);

    /* "/tmp" followed by slashes: 4095 bytes resolve, 4096 do not. */
    p = str_cpy(path, D);
    while (p - path < 4095) *p++ = '/';
    *p = 0;
    expect("path_4095_eexist", mk(path, S_IFIFO | 0644, 0), -EEXIST);
    *p++ = '/';
    *p = 0;
    expect("path_4096_enametoolong", mk(path, S_IFIFO | 0644, 0), -ENAMETOOLONG);

    expect("cleanup_v", a_unlink(D "/d/v"), 0);
    expect("cleanup_w", a_unlink(D "/w"), 0);
    expect("cleanup_s", a_unlink(D "/s"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_dl", a_unlink(D "/dl"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dirlink", a_unlink(D "/dirlink"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
