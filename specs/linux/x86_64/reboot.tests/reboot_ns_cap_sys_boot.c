/* The privilege reboot checks is CAP_SYS_BOOT (22) in the effective set:
 * root (pid 1 of a fresh pid namespace) that removes only that bit gets
 * EPERM (1) for RESTART and also for wrong magics (EPERM before EINVAL);
 * with the bit back, wrong magics are EINVAL (22) and RESTART ends the
 * namespace with wait status exactly SIGHUP (1).
 * poly-diff: needs=root (only root has CAP_SYS_BOOT to drop and restore)
 */
#include "poly_ipc.h"

#define SYS_reboot 169
/* linux/reboot.h */
#define MAGIC1 0xfee1deadUL
#define MAGIC2 672274793UL  /* 0x28121969 */
#define MAGIC2A 85072278UL  /* 0x05121996 */
#define MAGIC2B 369367448UL /* 0x16041998 */
#define MAGIC2C 537993216UL /* 0x20112000 */
#define CMD_RESTART 0x01234567UL
#define CMD_HALT 0xCDEF0123UL
#define CMD_CAD_ON 0x89ABCDEFUL
#define CMD_CAD_OFF 0x00000000UL
#define CMD_POWER_OFF 0x4321FEDCUL
#define CMD_RESTART2 0xA1B2C3D4UL
#define CMD_SW_SUSPEND 0xD000FCE1UL
#define CMD_KEXEC 0x45584543UL

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

/* Empties all three capability sets (any user may do that) and checks
 * that the effective set really is empty, so reboot can only be refused:
 * root then gets the same results as any other user. Returns 1 when the
 * drop is confirmed; the test must not call reboot otherwise. */
static __attribute__((unused)) int drop_all_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    i64 r = syscall2(SYS_capset, &h, d);
    check("drop_caps", r == 0, r);
    d[0].effective = d[1].effective = 0xffffffff;
    r = syscall2(SYS_capget, &h, d);
    int empty = r == 0 && d[0].effective == 0 && d[1].effective == 0;
    check("caps_empty", empty, r);
    return empty;
}

static __attribute__((unused)) i64 reboot4(u64 m1, u64 m2, u64 cmd, const void *arg) {
    return syscall4(SYS_reboot, m1, m2, cmd, arg);
}

#define CLONE_NEWPID 0x20000000 /* linux/sched.h */
#define CAP_SYS_BOOT 22          /* linux/capability.h */

/* Runs `body` as pid 1 of a new pid namespace, where reboot only ends
 * that namespace and never touches the machine. The child never calls
 * body unless it really is pid 1 of a fresh namespace. Returns the
 * child's raw wait status (-1 when clone or the wait failed). */
static __attribute__((unused)) int in_pid_ns(void (*body)(void)) {
    i64 pid = syscall5(SYS_clone, CLONE_NEWPID | SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        if (getpid() != 1) exit_group(100);
        body();
        exit_group(failed ? 1 : 0);
    }
    check("clone_newpid", pid > 0, pid);
    if (pid < 0) return -1;
    return wait_status(pid);
}
static volatile unsigned *flag;

static void body(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    syscall2(SYS_capget, &h, d);
    d[0].effective &= ~(1U << CAP_SYS_BOOT);
    i64 r = syscall2(SYS_capset, &h, d);
    check("drop_boot_effective", r == 0, r);
    if (r != 0) return;
    r = reboot4(MAGIC1, MAGIC2, CMD_RESTART, 0);
    check("without_boot_eperm", r == -EPERM, r);
    r = reboot4(1, 2, 3, 0);
    check("without_boot_bad_magic_eperm", r == -EPERM, r);
    d[0].effective |= 1U << CAP_SYS_BOOT;
    r = syscall2(SYS_capset, &h, d);
    check("restore_boot", r == 0, r);
    r = reboot4(1, 2, 3, 0);
    check("with_boot_bad_magic_einval", r == -EINVAL, r);
    flag[0] = 1;
    reboot4(MAGIC1, MAGIC2, CMD_RESTART, 0);
    flag[1] = 1;
}

int test_main(void) {
    flag = shared_page(4096);
    check("shared_page", flag != 0, 0);
    if (!flag) return finish();
    int s = in_pid_ns(body);
    check("reached_restart", flag[0] == 1, flag[0]);
    check("no_return", flag[1] == 0, flag[1]);
    check("sighup", s == SIGHUP, s);
    return finish();
}
