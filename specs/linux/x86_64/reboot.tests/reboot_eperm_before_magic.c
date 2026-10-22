/* The privilege check comes before the magic numbers are looked at:
 * without CAP_SYS_BOOT a wrong first magic, a wrong second magic, both
 * zero, or the two magics swapped are all EPERM (1), not EINVAL.
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
int test_main(void) {
    if (!drop_all_caps()) return finish();
    i64 r = reboot4(0x12345678, MAGIC2, CMD_RESTART, 0);
    check("bad_magic1_eperm", r == -EPERM, r);
    r = reboot4(MAGIC1, 0x12345678, CMD_RESTART, 0);
    check("bad_magic2_eperm", r == -EPERM, r);
    r = reboot4(0, 0, 0, 0);
    check("all_zero_eperm", r == -EPERM, r);
    r = reboot4(MAGIC2, MAGIC1, CMD_HALT, 0);
    check("swapped_eperm", r == -EPERM, r);
    r = reboot4(0xffffffffUL, 0xffffffffUL, 0xffffffffUL, 0);
    check("all_ones_eperm", r == -EPERM, r);
    return finish();
}
