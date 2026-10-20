/* fchownat: AT_SYMLINK_NOFOLLOW changes the link itself; without it the
 * target changes; AT_EMPTY_PATH and fchown work on descriptors. */
#include "a_sys.h"

#define D "/tmp/a_fchownat_flags"

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);
    expect("chown_through_link", syscall5(SYS_fchownat, AT_FDCWD, D "/l", 11, 12, 0), 0);
    a_stat_path(D "/f", &st);
    check("target_changed", st.st_uid == 11 && st.st_gid == 12, st.st_uid);
    a_lstat_path(D "/l", &st);
    check("link_unchanged", st.st_uid == 0 && st.st_gid == 0, st.st_uid);
    expect("chown_link_nofollow", syscall5(SYS_fchownat, AT_FDCWD, D "/l", 21, 22, AT_SYMLINK_NOFOLLOW), 0);
    a_lstat_path(D "/l", &st);
    check("link_changed", st.st_uid == 21 && st.st_gid == 22, st.st_uid);
    a_stat_path(D "/f", &st);
    check("target_kept", st.st_uid == 11, st.st_uid);
    expect("dangling", syscall3(SYS_symlinkat, "gone", AT_FDCWD, D "/d"), 0);
    expect("dangling_follow_enoent", syscall5(SYS_fchownat, AT_FDCWD, D "/d", 1, 1, 0), -ENOENT);
    expect("dangling_nofollow_ok", syscall5(SYS_fchownat, AT_FDCWD, D "/d", 1, 1, AT_SYMLINK_NOFOLLOW), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("empty_path_fd", syscall5(SYS_fchownat, fd, "", 31, 32, AT_EMPTY_PATH), 0);
    syscall2(SYS_fstat, fd, &st);
    check("empty_path_result", st.st_uid == 31 && st.st_gid == 32, st.st_uid);
    expect("empty_path_without_flag_enoent", syscall5(SYS_fchownat, fd, "", 0, 0, 0), -ENOENT);
    expect("fchown_fd", syscall3(SYS_fchown, fd, 41, -1), 0);
    syscall2(SYS_fstat, fd, &st);
    check("fchown_result", st.st_uid == 41 && st.st_gid == 32, st.st_uid);
    a_close(fd);
    expect("fchown_bad_fd_ebadf", syscall3(SYS_fchown, 99, 0, 0), -EBADF);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("relative", syscall5(SYS_fchownat, dfd, "f", 0, 0, 0), 0);
    expect("bad_dirfd_ebadf", syscall5(SYS_fchownat, 99, "f", 0, 0, 0), -EBADF);
    a_close(dfd);
    expect("cleanup_d", a_unlink(D "/d"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
