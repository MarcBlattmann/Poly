/* A caller without privilege may not give its file away or move it to a
 * group it is not in: another uid, a gid that is neither its effective
 * gid nor one of its supplementary groups, gid 0, or uid 0xfffffffe all
 * fail with EPERM, also when the other id is acceptable; nothing changes
 * (owner, group, set-user-ID and set-group-ID bits stay). The checks run
 * in a child; a root caller first gives up root (to uid and gid 65534, no
 * supplementary groups). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getgroups 115
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119

#define F "/tmp/c_fchown_foreign_ids"

static i64 fstat_field(i64 fd, int which) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    if (r < 0) return r;
    return which == 0 ? (i64)st.st_uid : which == 1 ? (i64)st.st_gid : (i64)(st.st_mode & 07777);
}

/* A gid the caller is not in: not its effective gid, not supplementary. */
static i64 foreign_gid(i64 gid) {
    i32 groups[256];
    i64 n = syscall2(SYS_getgroups, 256, groups);
    for (i64 cand = gid + 1;; cand++) {
        int member = 0;
        for (i64 i = 0; i < n; i++)
            if (groups[i] == cand) member = 1;
        if (!member && cand != 0) return cand;
    }
}

static int body(void) {
    if (syscall0(SYS_geteuid) == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 65534, 65534, 65534);
        syscall3(SYS_setresuid, 65534, 65534, 65534);
    }
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    check("not_root", uid != 0, uid);
    if (uid == 0) return finish();
    i64 other = foreign_gid(gid);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_EXCL, 0644);
    check("setup_open", fd >= 0, fd);
    expect("chmod_06755", syscall2(SYS_fchmod, fd, 06755), 0);
    expect("other_uid", syscall3(SYS_fchown, fd, uid + 1, -1), -EPERM);
    expect("uid_zero", syscall3(SYS_fchown, fd, 0, -1), -EPERM);
    expect("uid_fffffffe", syscall3(SYS_fchown, fd, 0xfffffffeL, -1), -EPERM);
    expect("other_uid_own_gid", syscall3(SYS_fchown, fd, uid + 1, gid), -EPERM);
    expect("foreign_gid", syscall3(SYS_fchown, fd, -1, other), -EPERM);
    expect("gid_zero", syscall3(SYS_fchown, fd, -1, 0), -EPERM);
    expect("own_uid_foreign_gid", syscall3(SYS_fchown, fd, uid, other), -EPERM);
    expect("both_foreign", syscall3(SYS_fchown, fd, uid + 1, other), -EPERM);
    expect("uid_kept", fstat_field(fd, 0), uid);
    expect("gid_kept", fstat_field(fd, 1), gid);
    expect("mode_kept", fstat_field(fd, 2), 06755);
    expect("own_ids_still_ok", syscall3(SYS_fchown, fd, uid, gid), 0);
    expect("mode_after_ok", fstat_field(fd, 2), 00755);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}

int test_main(void) {
    a_unlink(F);
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, body());
    i32 status = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return 1;
    return A_WIFEXITED(status) ? A_WEXITSTATUS(status) : 1;
}
