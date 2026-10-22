/* lchown(2) works on every kind of file the caller owns: regular files,
 * directories, FIFOs, sockets and symbolic links. Ownership belongs to the
 * inode: a hard link sees the change (and the ctime) made through the
 * other name, and so does a descriptor opened before the call. The
 * set-ID rules for regular files apply to FIFOs and sockets as well. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_lchown 94
#define SYS_mknodat 259
#define D "/tmp/a_lchown_types"

static i64 lchown(const char *p, i64 uid, i64 gid) { return syscall3(SYS_lchown, p, uid, gid); }

static void nap(void) {
    struct a_timespec t = {0, 30000000};
    syscall2(SYS_nanosleep, &t, 0);
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    struct a_stat st, a, b;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 06755, 0), 0);
    expect("setup_sock", syscall4(SYS_mknodat, AT_FDCWD, D "/sock", S_IFSOCK | 06745, 0), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_hardlink", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/h", 0), 0);

    expect("fifo", lchown(D "/fifo", uid, gid), 0);
    a_lstat_path(D "/fifo", &st);
    check("fifo_still_fifo", (st.st_mode & S_IFMT) == S_IFIFO, st.st_mode);
    check("fifo_ids", st.st_uid == (u32)uid && st.st_gid == (u32)gid, st.st_uid);
    expect("fifo_setid_cleared", st.st_mode & 07777, 0755);
    expect("sock", lchown(D "/sock", -1, -1), 0);
    a_lstat_path(D "/sock", &st);
    check("sock_still_sock", (st.st_mode & S_IFMT) == S_IFSOCK, st.st_mode);
    expect("sock_suid_cleared_sgid_kept", st.st_mode & 07777, 02745);

    i64 fd = a_open(D "/f", O_RDWR, 0);
    check("open_file", fd >= 0, fd);
    expect("setup_suid", syscall2(SYS_chmod, D "/f", 04755), 0);
    a_lstat_path(D "/h", &a);
    nap();
    expect("via_first_name", lchown(D "/f", uid, gid), 0);
    a_lstat_path(D "/h", &b);
    check("hardlink_ctime_changed", b.st_ctime_sec != a.st_ctime_sec || b.st_ctime_nsec != a.st_ctime_nsec, 0);
    expect("hardlink_bits_cleared", b.st_mode & 07777, 0755);
    expect("hardlink_nlink", b.st_nlink, 2);
    syscall2(SYS_fstat, fd, &st);
    expect("open_fd_sees_mode", st.st_mode & 07777, 0755);
    check("open_fd_same_ctime", st.st_ctime_sec == b.st_ctime_sec && st.st_ctime_nsec == b.st_ctime_nsec, 0);
    /* The descriptor still works. */
    expect("open_fd_write", a_write(fd, "x", 1), 1);
    a_close(fd);

    expect("via_second_name", lchown(D "/h", -1, -1), 0);

    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sock", a_unlink(D "/sock"), 0);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
