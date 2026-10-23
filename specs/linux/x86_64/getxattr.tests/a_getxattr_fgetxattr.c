/* fgetxattr (193) reads through an open descriptor of a file or directory,
 * whatever its access mode (O_RDONLY, O_WRONLY, O_RDWR, O_DIRECTORY); the
 * permission check uses the file's mode, not the descriptor (all
 * capabilities dropped first: a 0200 file opened O_WRONLY is EACCES). An
 * unlinked but open file keeps its attributes. O_PATH, closed, never
 * opened and negative descriptors are EBADF; AT_FDCWD (-100) acts on the
 * working directory. Name errors (ERANGE, EFAULT) come before EBADF. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_fgetxattr 193
#define ENODATA 61
#define EBADF 9
#define D "/tmp/a_getxattr_fgetxattr"
#define BAD ((const void *)8)

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 fget(i64 fd, const void *name) { return syscall4(SYS_fgetxattr, fd, name, buf, sizeof buf); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "abc", 3, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D, "user.d", "dd", 2, 0), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    expect("rdonly", fget(fd, "user.a"), 3);
    check("rdonly_value", mem_eq(buf, "abc", 3), 0);
    expect("rdonly_size0", syscall4(SYS_fgetxattr, fd, "user.a", 0, 0), 3);
    expect("rdonly_erange", syscall4(SYS_fgetxattr, fd, "user.a", buf, 2), -ERANGE);
    expect("rdonly_missing", fget(fd, "user.b"), -ENODATA);
    expect("rdonly_unknown", fget(fd, "foo.a"), -EOPNOTSUPP);
    a_close(fd);
    fd = a_open(D "/f", O_WRONLY, 0);
    expect("wronly", fget(fd, "user.a"), 3);
    a_close(fd);
    fd = a_open(D "/f", O_RDWR, 0);
    expect("rdwr", fget(fd, "user.a"), 3);
    expect("unlink_open", a_unlink(D "/f"), 0);
    expect("unlinked_still_readable", fget(fd, "user.a"), 3);
    a_close(fd);
    fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("directory", fget(fd, "user.d"), 2);
    a_close(fd);
    fd = a_open(D, O_PATH, 0);
    check("setup_opath", fd >= 0, fd);
    expect("opath_ebadf", fget(fd, "user.d"), -EBADF);
    a_close(fd);
    expect("closed_ebadf", fget(fd, "user.d"), -EBADF);
    expect("never_opened_ebadf", fget(999, "user.d"), -EBADF);
    expect("negative_ebadf", fget(-1, "user.d"), -EBADF);
    expect("empty_name_before_ebadf", fget(999, ""), -ERANGE);
    expect("bad_name_before_ebadf", fget(999, BAD), -EFAULT);
    expect("ebadf_before_unknown_prefix", fget(999, "foo.a"), -EBADF);
    expect("ebadf_before_user_dot", fget(999, "user."), -EBADF);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("at_fdcwd_cwd", fget(AT_FDCWD, "user.d"), 2);
    expect("chdir_back", syscall1(SYS_chdir, "/"), 0);
    expect("setup_wo", a_make_file(D "/wo", "", 0644), 0);
    expect("setup_set_wo", syscall5(SYS_setxattr, D "/wo", "user.a", "w", 1, 0), 0);
    expect("setup_wo_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/wo", 0200), 0);
    fd = a_open(D "/wo", O_WRONLY, 0);
    expect("mode_decides_eacces", fget(fd, "user.a"), -EACCES);
    expect("mode_decides_trusted_enodata", fget(fd, "trusted.a"), -ENODATA);
    a_close(fd);
    expect("cleanup_wo", a_unlink(D "/wo"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
