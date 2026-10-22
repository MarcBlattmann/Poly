/* fchown on a file the caller owns: -1 for both ids, or the caller's own
 * uid and gid, returns 0 and leaves the owner and group as they were, seen
 * through the same descriptor, another descriptor and the path. The
 * descriptor may be open read-only or write-only; no access to the file
 * is needed (a file with mode 0000 still works). */
#include "a_sys.h"

#define D "/tmp/c_fchown_basic"

static i64 fuid(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_uid;
}

static i64 fgid(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_gid;
}

int test_main(void) {
    struct a_stat st;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    a_unlink(D "/f");
    a_rmdir(D);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 fd = a_open(D "/f", O_RDWR | O_CREAT | O_EXCL, 0644);
    check("setup_open", fd >= 0, fd);
    expect("owner_is_caller", fuid(fd), uid);
    expect("group_is_caller", fgid(fd), gid);
    expect("both_minus_one", syscall3(SYS_fchown, fd, -1, -1), 0);
    expect("uid_kept", fuid(fd), uid);
    expect("gid_kept", fgid(fd), gid);
    expect("own_ids", syscall3(SYS_fchown, fd, uid, gid), 0);
    expect("own_uid_only", syscall3(SYS_fchown, fd, uid, -1), 0);
    expect("own_gid_only", syscall3(SYS_fchown, fd, -1, gid), 0);
    expect("uid_still", fuid(fd), uid);
    expect("gid_still", fgid(fd), gid);
    expect("path_uid", a_stat_path(D "/f", &st) == 0 ? (i64)st.st_uid : -1, uid);
    expect("path_gid", (i64)st.st_gid, gid);

    i64 rd = a_open(D "/f", O_RDONLY, 0);
    check("open_rdonly", rd >= 0, rd);
    expect("rdonly_fd", syscall3(SYS_fchown, rd, uid, gid), 0);
    a_close(rd);
    i64 wr = a_open(D "/f", O_WRONLY, 0);
    check("open_wronly", wr >= 0, wr);
    expect("wronly_fd", syscall3(SYS_fchown, wr, -1, gid), 0);
    a_close(wr);

    expect("chmod_0000", syscall2(SYS_fchmod, fd, 0), 0);
    expect("mode_0000_fchown", syscall3(SYS_fchown, fd, uid, gid), 0);
    expect("mode_0000_kept", syscall2(SYS_fstat, fd, &st) == 0 ? (i64)(st.st_mode & 07777) : -1, 0);
    i64 dup = syscall1(SYS_dup, fd);
    expect("dup_fd", syscall3(SYS_fchown, dup, -1, -1), 0);
    a_close(dup);
    a_close(fd);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
