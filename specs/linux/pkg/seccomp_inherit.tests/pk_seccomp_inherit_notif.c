/* SECCOMP_RET_USER_NOTIF with a listener: a filter installed with
 * SECCOMP_FILTER_FLAG_NEW_LISTENER returns a notification descriptor; a
 * supervisor (here the same process, which never makes the intercepted
 * call itself) reads each intercepted call of the tasks under the filter
 * with ioctl(SECCOMP_IOCTL_NOTIF_RECV, struct seccomp_notif) - an id, the
 * caller's pid and a copy of seccomp_data - and answers it with
 * SECCOMP_IOCTL_NOTIF_SEND (struct seccomp_notif_resp {id, val, error,
 * flags}): val becomes the call's result, a negative error (errno) becomes
 * -errno, and the flag SECCOMP_USER_NOTIF_FLAG_CONTINUE (1) lets the call
 * run as if the filter had allowed it. SECCOMP_IOCTL_NOTIF_ID_VALID
 * (with the id) is 0 while the caller waits and ENOENT once it has been
 * answered. The blocked caller sleeps until it is answered. RECV needs the
 * buffer zeroed (EINVAL otherwise, without waiting).
 * poly-diff: syscalls=off (two processes)
 */
#include "p_seccomp.h"

#define SECCOMP_IOCTL_NOTIF_RECV 0xc0502100UL
#define SECCOMP_IOCTL_NOTIF_SEND 0xc0182101UL
#define SECCOMP_IOCTL_NOTIF_ID_VALID 0x40082102UL
#define SECCOMP_USER_NOTIF_FLAG_CONTINUE 1

struct notif { /* struct seccomp_notif, 80 bytes */
    u64 id;
    u32 pid, flags;
    i32 nr;
    u32 arch;
    u64 ip;
    u64 args[6];
};
struct resp { /* struct seccomp_notif_resp, 24 bytes */
    u64 id;
    i64 val;
    i32 error;
    u32 flags;
};

static int child_main(void) {
    int bad = 0;
    if (syscall0(SYS_sched_yield) != 1234) bad |= 1;
    if (syscall0(SYS_sched_yield) != -EPERM) bad |= 2;
    if (syscall0(SYS_sched_yield) != 0) bad |= 4; /* continued: really ran */
    if (syscall0(SYS_getpid) <= 0) bad |= 8;      /* not intercepted */
    return bad;
}

int test_main(void) {
    a_block_sigchld();
    struct pk_insn f[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_sched_yield, 0, 1), PK_RET(SECCOMP_RET_USER_NOTIF), PK_ALLOW};
    struct pk_prog p = {4, f};
    syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    i64 fd = syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_NEW_LISTENER, &p);
    check("install_with_listener", fd >= 0, fd);
    if (fd < 0) return finish();
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) syscall1(SYS_exit_group, child_main());

    for (int i = 0; i < 3; i++) {
        struct notif nt;
        memset(&nt, 0, sizeof nt);
        expect("recv", syscall3(SYS_ioctl, fd, SECCOMP_IOCTL_NOTIF_RECV, &nt), 0);
        expect("notif_nr", nt.nr, SYS_sched_yield);
        expect("notif_arch", nt.arch, AUDIT_ARCH_X86_64);
        check("notif_pid_is_the_child", nt.pid != 0 && (i64)nt.pid == pid, nt.pid);
        expect("notif_flags_zero", nt.flags, 0);
        expect("id_valid_while_waiting", syscall3(SYS_ioctl, fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &nt.id), 0);
        struct resp rp;
        memset(&rp, 0, sizeof rp);
        rp.id = nt.id;
        if (i == 0) {
            rp.val = 1234;
        } else if (i == 1) {
            rp.error = -EPERM;
        } else {
            rp.flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE;
        }
        expect("send", syscall3(SYS_ioctl, fd, SECCOMP_IOCTL_NOTIF_SEND, &rp), 0);
        expect("id_invalid_after_answer", syscall3(SYS_ioctl, fd, SECCOMP_IOCTL_NOTIF_ID_VALID, &nt.id), -ENOENT);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_saw_value_error_and_continue", st, 0);
    /* the buffer must be zero */
    struct notif nt;
    memset(&nt, 0xff, sizeof nt);
    expect("recv_with_dirty_buffer_einval", syscall3(SYS_ioctl, fd, SECCOMP_IOCTL_NOTIF_RECV, &nt), -EINVAL);
    return finish();
}
