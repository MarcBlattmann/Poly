/* fchmodat: sets the 12 permission bits exactly (the umask does not
 * apply); type bits in the mode are ignored; fchmod on a descriptor. */
#include "a_sys.h"

#define D "/tmp/a_fchmodat_bits"

static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    i64 old = syscall1(SYS_umask, 077);
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "", 0600), 0);
    expect("chmod_0777", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 0777), 0);
    expect("umask_not_applied", mode_of(D "/f"), S_IFREG | 0777);
    expect("chmod_0", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 0), 0);
    expect("mode_0", mode_of(D "/f"), S_IFREG);
    expect("chmod_suid_sgid_sticky", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 07777), 0);
    expect("all_12_bits_as_root", mode_of(D "/f"), S_IFREG | 07777);
    expect("chmod_type_bits_ignored", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", S_IFDIR | 0640), 0);
    expect("still_regular", mode_of(D "/f"), S_IFREG | 0640);
    expect("chmod_high_bits_ignored", syscall3(SYS_fchmodat, AT_FDCWD, D "/f", 0xffff0000 | 0604), 0);
    expect("high_bits_dropped", mode_of(D "/f"), S_IFREG | 0604);
    expect("chmod_dir", syscall3(SYS_fchmodat, AT_FDCWD, D, 01755), 0);
    expect("dir_mode", mode_of(D), S_IFDIR | 01755);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("fchmod_fd", syscall2(SYS_fchmod, fd, 0421), 0);
    expect("fchmod_result", mode_of(D "/f"), S_IFREG | 0421);
    a_close(fd);
    expect("fchmod_bad_fd_ebadf", syscall2(SYS_fchmod, 99, 0600), -EBADF);
    fd = a_open(D "/f", O_PATH, 0);
    expect("fchmod_path_fd_ebadf", syscall2(SYS_fchmod, fd, 0600), -EBADF);
    a_close(fd);
    expect("old_chmod_syscall", syscall2(SYS_chmod, D "/f", 0640), 0);
    expect("old_chmod_result", mode_of(D "/f"), S_IFREG | 0640);
    syscall1(SYS_umask, old);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
