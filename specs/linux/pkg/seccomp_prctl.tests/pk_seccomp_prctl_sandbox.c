/* The order of steps a sandbox such as bubblewrap takes before it runs the
 * app, ending with prctl(PR_SET_SECCOMP): clear the dumpable flag, set
 * no_new_privs, drop every capability from the bounding set, install a
 * deny-list filter (privileged system calls fail with EPERM), and then
 * start the app as a child. Afterwards the denied calls return -EPERM
 * without running (with arguments that would otherwise be EFAULT), the
 * other calls work, and a child made by fork sees the same dumpable flag
 * (0), no_new_privs (1), empty bounding set, seccomp mode (2) and filter.
 * The filter cannot be undone in the process: PR_SET_SECCOMP mode 0 is
 * EINVAL, and installing an allow-all filter later changes nothing.
 * The denied calls are mount, umount2, pivot_root, chroot, ptrace, setns,
 * unshare and the keyring calls, by number.
 * poly-diff: needs=root
 * poly-diff: syscalls=off (the test forks)
 */
#include "p_seccomp.h"

#define SYS_mount 165
#define SYS_umount2 166
#define SYS_pivot_root 155
#define SYS_chroot 161
#define SYS_ptrace 101
#define SYS_setns 308
#define SYS_unshare 272
#define SYS_keyctl 250
#define P(o, a, b, c, d) syscall5(SYS_prctl, (o), (a), (b), (c), (d))

static const i64 denied[] = {SYS_mount, SYS_umount2, SYS_pivot_root, SYS_chroot, SYS_ptrace, SYS_setns, SYS_unshare, SYS_keyctl};

static void report_denied(const char *prefix) {
    char name[64];
    for (u32 i = 0; i < sizeof denied / sizeof denied[0]; i++) {
        char num[24];
        str_cat2(name, prefix, a_utoa(num, (u64)denied[i]));
        expect(name, syscall6(denied[i], 0, 0, 0, 0, 0, 0), -EPERM);
    }
}

static int caps_empty(void) {
    for (i64 c = 0; P(PR_CAPBSET_READ, c, 0, 0, 0) >= 0; c++)
        if (P(PR_CAPBSET_READ, c, 0, 0, 0) != 0) return 0;
    return 1;
}

static void sandbox(void) {
    expect("set_dumpable_0", P(PR_SET_DUMPABLE, 0, 0, 0, 0), 0);
    expect("set_no_new_privs", P(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), 0);
    int dropped = 1;
    for (i64 c = 0; P(PR_CAPBSET_READ, c, 0, 0, 0) >= 0; c++)
        if (P(PR_CAPBSET_DROP, c, 0, 0, 0) != 0) dropped = 0;
    check("bounding_set_dropped", dropped, 0);

    /* deny list: EPERM for the privileged calls, everything else allowed */
    struct pk_insn f[24];
    int n = 0;
    f[n++] = (struct pk_insn)PK_LDABS(SD_ARCH);
    f[n++] = (struct pk_insn)PK_JEQ(AUDIT_ARCH_X86_64, 1, 0);
    f[n++] = (struct pk_insn)PK_RET(SECCOMP_RET_KILL_PROCESS);
    f[n++] = (struct pk_insn)PK_LDABS(SD_NR);
    for (u32 i = 0; i < sizeof denied / sizeof denied[0]; i++) {
        /* equal -> jump to the shared ERR(1) at the end; else next comparison */
        u32 remaining = (u32)(sizeof denied / sizeof denied[0]) - i - 1;
        f[n++] = (struct pk_insn)PK_JEQ((u32)denied[i], (u8)(remaining + 1), 0);
    }
    f[n++] = (struct pk_insn)PK_ALLOW;
    f[n++] = (struct pk_insn)PK_ERR(1);
    struct pk_prog p = {(u16)n, f};
    expect("install_with_prctl", P(PR_SET_SECCOMP, 2, &p, 0, 0), 0);
    expect("mode_2", P(PR_GET_SECCOMP, 0, 0, 0, 0), 2);
    expect("mode_0_still_einval", P(PR_SET_SECCOMP, 0, &p, 0, 0), -EINVAL);

    report_denied("denied_");
    check("getpid_works", syscall0(SYS_getpid) > 0, 0);
    expect("sched_yield_works", syscall0(SYS_sched_yield), 0);
    expect("dumpable_still_0", P(PR_GET_DUMPABLE, 0, 0, 0, 0), 0);
    expect("nnp_still_1", P(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 1);
    check("bounding_set_empty", caps_empty(), 0);

    /* an allow-all filter added later removes nothing */
    struct pk_insn allow[] = {PK_ALLOW};
    struct pk_prog pa = {1, allow};
    expect("install_allow_all_later", P(PR_SET_SECCOMP, 2, &pa, 0, 0), 0);
    expect("mount_still_denied", syscall6(SYS_mount, 0, 0, 0, 0, 0, 0), -EPERM);

    /* the app: a fork child */
    i64 pid = a_fork_sync();
    if (pid == 0) {
        int ok = 1;
        if (P(PR_GET_DUMPABLE, 0, 0, 0, 0) != 0) ok = 0;
        if (P(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 1) ok = 0;
        if (P(PR_GET_SECCOMP, 0, 0, 0, 0) != 2) ok = 0;
        if (!caps_empty()) ok = 0;
        for (u32 i = 0; i < sizeof denied / sizeof denied[0]; i++)
            if (syscall6(denied[i], 0, 0, 0, 0, 0, 0) != -EPERM) ok = 0;
        if (syscall0(SYS_sched_yield) != 0) ok = 0;
        syscall1(SYS_exit_group, ok ? 0 : 1);
    }
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_sees_everything", st, 0);
}

int test_main(void) {
    a_block_sigchld();
    pk_in_child(sandbox);
    /* nothing of it reached the parent */
    expect("parent_dumpable_unchanged", P(PR_GET_DUMPABLE, 0, 0, 0, 0), 1);
    expect("parent_nnp_unchanged", P(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), 0);
    expect("parent_bounding_set_unchanged", P(PR_CAPBSET_READ, 21, 0, 0, 0), 1);
    return finish();
}
