/* newfstatat: AT_EMPTY_PATH on a descriptor, AT_NO_AUTOMOUNT accepted,
 * unknown flags refused. */
#include "a_sys.h"

#define D "/tmp/a_newfstatat_flags"

static i64 fstatat(i64 dfd, const char *p, struct a_stat *st, i64 flags) {
    return syscall4(SYS_newfstatat, dfd, p, st, flags);
}

int test_main(void) {
    struct a_stat st, ref;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "abc", 0600), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    syscall2(SYS_fstat, fd, &ref);
    expect("empty_path_on_fd", fstatat(fd, "", &st, AT_EMPTY_PATH), 0);
    check("empty_path_same_as_fstat", st.st_ino == ref.st_ino && st.st_size == 3 && st.st_mode == ref.st_mode, 0);
    expect("empty_path_without_flag_enoent", fstatat(fd, "", &st, 0), -ENOENT);
    expect("empty_path_cwd", fstatat(AT_FDCWD, "", &st, AT_EMPTY_PATH), 0);
    expect("empty_path_cwd_is_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("empty_path_bad_fd_ebadf", fstatat(99, "", &st, AT_EMPTY_PATH), -EBADF);
    expect("nonempty_path_with_empty_flag", fstatat(AT_FDCWD, D "/f", &st, AT_EMPTY_PATH), 0);
    expect("no_automount_accepted", fstatat(AT_FDCWD, D "/f", &st, AT_NO_AUTOMOUNT), 0);
    expect("unknown_flag_einval", fstatat(AT_FDCWD, D "/f", &st, 0x1), -EINVAL);
    expect("removedir_flag_einval", fstatat(AT_FDCWD, D "/f", &st, AT_REMOVEDIR), -EINVAL);
    expect("follow_flag_einval", fstatat(AT_FDCWD, D "/f", &st, AT_SYMLINK_FOLLOW), -EINVAL);
    a_close(fd);
    i64 p = a_open(D "/f", O_PATH, 0);
    expect("empty_path_on_path_fd", fstatat(p, "", &st, AT_EMPTY_PATH), 0);
    expect("path_fd_size", st.st_size, 3);
    a_close(p);
    /* The old stat and lstat calls agree with newfstatat. */
    expect("stat_syscall", syscall2(SYS_stat, D "/f", &st), 0);
    check("stat_same_inode", st.st_ino == ref.st_ino, 0);
    expect("lstat_syscall", syscall2(SYS_lstat, D, &st), 0);
    expect("lstat_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
