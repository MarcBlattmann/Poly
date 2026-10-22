/* fchown on pipe ends, a Unix socket and a memfd: they belong to the
 * caller (fstat shows its uid and gid), -1 for both ids and the caller's
 * own ids succeed, another uid fails with EPERM for a caller without
 * privilege. A memfd is a regular file: set-user-ID and set-group-ID
 * (with group execute) are cleared as on any regular file. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_socket 41
#define SYS_memfd_create 319

#define AF_UNIX 1
#define SOCK_STREAM 1

static i64 field(i64 fd, int which) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    if (r < 0) return r;
    return which == 0 ? (i64)st.st_uid : which == 1 ? (i64)st.st_gid : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    i32 p[2];
    expect("pipe", syscall2(SYS_pipe2, p, 0), 0);
    expect("pipe_owner", field(p[0], 0), uid);
    expect("pipe_group", field(p[0], 1), gid);
    expect("pipe_read_minus_one", syscall3(SYS_fchown, p[0], -1, -1), 0);
    expect("pipe_write_own_ids", syscall3(SYS_fchown, p[1], uid, gid), 0);
    if (uid != 0) expect("pipe_foreign_uid", syscall3(SYS_fchown, p[0], uid + 1, -1), -EPERM);
    a_close(p[0]);
    a_close(p[1]);

    i64 s = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    check("socket", s >= 0, s);
    expect("socket_owner", field(s, 0), uid);
    expect("socket_minus_one", syscall3(SYS_fchown, s, -1, -1), 0);
    expect("socket_own_ids", syscall3(SYS_fchown, s, uid, gid), 0);
    if (uid != 0) expect("socket_foreign_uid", syscall3(SYS_fchown, s, uid + 1, -1), -EPERM);
    a_close(s);

    i64 m = syscall2(SYS_memfd_create, "c_fchown", 0);
    check("memfd", m >= 0, m);
    expect("memfd_owner", field(m, 0), uid);
    expect("memfd_minus_one", syscall3(SYS_fchown, m, -1, -1), 0);
    expect("memfd_own_ids", syscall3(SYS_fchown, m, uid, gid), 0);
    if (uid != 0) expect("memfd_foreign_uid", syscall3(SYS_fchown, m, uid + 1, -1), -EPERM);
    expect("memfd_chmod", syscall2(SYS_fchmod, m, 06755), 0);
    expect("memfd_clear", syscall3(SYS_fchown, m, -1, -1), 0);
    expect("memfd_mode", field(m, 2), 00755);
    a_close(m);
    return finish();
}
