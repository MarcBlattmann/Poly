/* Inside a non-initial pid namespace, reboot(HALT) and reboot(POWER_OFF)
 * do not return and end the namespace's init as if killed by SIGINT:
 * the parent's wait status is exactly 2. One namespace per op.
 * poly-diff: needs=root (only root has CAP_SYS_BOOT to reboot a pid namespace)
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
static u64 op;

static void body(void) {
    reboot4(MAGIC1, MAGIC2, op, 0);
    flag[0]++;
}

int test_main(void) {
    flag = shared_page(4096);
    check("shared_page", flag != 0, 0);
    if (!flag) return finish();
    op = CMD_HALT;
    int s = in_pid_ns(body);
    check("halt_sigint", s == SIGINT, s);
    op = CMD_POWER_OFF;
    s = in_pid_ns(body);
    check("power_off_sigint", s == SIGINT, s);
    check("no_return", flag[0] == 0, flag[0]);
    return finish();
}
