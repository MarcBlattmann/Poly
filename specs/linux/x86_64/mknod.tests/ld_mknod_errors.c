/* mknod errors and their order. The file type is checked first, before
 * the path is read: S_IFDIR is EPERM (use mkdir), S_IFLNK and the unused
 * type values (e.g. 0030000, 0170000) are EINVAL - even with a NULL path
 * or a path in a missing directory. Then the path: EFAULT, ENAMETOOLONG
 * (a component over 255 bytes, a path of 4096 bytes or more), ENOENT (a
 * missing directory, an empty path, a trailing slash on a new
 * non-directory name), ENOTDIR (a file used as a directory), ELOOP
 * (symlink loop in the directory part), EEXIST (the name exists, also as
 * a dangling symlink, which is not followed, and for "." or "..").
 */
#include "a_sys.h"

#define SYS_mknod 133
#define DIR "/tmp/ld_mknod_errors"

static char longname[300], longpath[4200];

static i64 mk(const void *path, u64 mode) { return syscall3(SYS_mknod, path, mode, 0); }

static void cleanup(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/dangling");
    a_unlink(DIR "/loop");
    a_unlink(DIR "/ok255");
    a_rmdir(DIR);
}

int test_main(void) {
    cleanup();
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("file", a_make_file(DIR "/f", "x", 0644), 0);
    syscall2(SYS_symlink, DIR "/nowhere", DIR "/dangling");
    syscall2(SYS_symlink, "loop", DIR "/loop");

    expect("dir_type_eperm", mk(DIR "/d", S_IFDIR | 0755), -EPERM);
    expect("dir_type_null_path_eperm", mk(0, S_IFDIR | 0755), -EPERM);
    expect("dir_type_missing_dir_eperm", mk(DIR "/none/d", S_IFDIR | 0755), -EPERM);
    expect("dir_type_existing_eperm", mk(DIR "/f", S_IFDIR | 0755), -EPERM);
    expect("symlink_type_einval", mk(DIR "/l", S_IFLNK | 0777), -EINVAL);
    expect("symlink_type_null_path_einval", mk(0, S_IFLNK | 0777), -EINVAL);
    expect("type_0030000_einval", mk(DIR "/t", 0030000 | 0644), -EINVAL);
    expect("type_0170000_einval", mk(DIR "/t", 0170000 | 0644), -EINVAL);
    expect("type_0070000_missing_dir_einval", mk(DIR "/none/t", 0070000 | 0644), -EINVAL);

    expect("null_path_efault", mk(0, S_IFIFO | 0644), -EFAULT);
    expect("kernel_address_efault", mk((void *)0xffff800000000000UL, S_IFIFO | 0644), -EFAULT);
    for (int i = 0; i < 255; i++) longname[i] = 'n';
    str_cpy(longpath, DIR "/");
    str_cpy(longpath + str_len(longpath), longname);
    expect("name_255_ok", mk(longpath, S_IFIFO | 0644), 0);
    a_unlink(longpath);
    longname[255] = 'n';
    str_cpy(longpath, DIR "/");
    str_cpy(longpath + str_len(longpath), longname);
    expect("name_256_enametoolong", mk(longpath, S_IFIFO | 0644), -ENAMETOOLONG);
    for (int i = 0; i < 4100; i++) longpath[i] = (i % 2) ? '/' : 'a';
    longpath[4100] = 0;
    expect("path_4100_enametoolong", mk(longpath, S_IFIFO | 0644), -ENAMETOOLONG);
    expect("long_path_dir_type_eperm", mk(longpath, S_IFDIR | 0644), -EPERM);

    expect("missing_dir_enoent", mk(DIR "/none/x", S_IFIFO | 0644), -ENOENT);
    expect("empty_path_enoent", mk("", S_IFIFO | 0644), -ENOENT);
    expect("trailing_slash_enoent", mk(DIR "/new/", S_IFIFO | 0644), -ENOENT);
    expect("trailing_slash_regular_enoent", mk(DIR "/new/", S_IFREG | 0644), -ENOENT);
    expect("file_as_dir_enotdir", mk(DIR "/f/x", S_IFIFO | 0644), -ENOTDIR);
    expect("loop_in_dir_part_eloop", mk(DIR "/loop/x", S_IFIFO | 0644), -ELOOP);
    expect("exists_eexist", mk(DIR "/f", S_IFIFO | 0644), -EEXIST);
    expect("exists_regular_eexist", mk(DIR "/f", S_IFREG | 0644), -EEXIST);
    expect("dangling_symlink_eexist", mk(DIR "/dangling", S_IFIFO | 0644), -EEXIST);
    struct a_stat st;
    expect("dangling_target_not_created", a_stat_path(DIR "/nowhere", &st), -ENOENT);
    expect("loop_last_eexist", mk(DIR "/loop", S_IFIFO | 0644), -EEXIST);
    expect("dot_eexist", mk(DIR "/.", S_IFIFO | 0644), -EEXIST);
    expect("dotdot_eexist", mk(DIR "/..", S_IFIFO | 0644), -EEXIST);
    expect("existing_dir_eexist", mk("/tmp", S_IFIFO | 0644), -EEXIST);

    cleanup();
    return finish();
}
