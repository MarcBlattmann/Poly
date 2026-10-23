/* A fork child of a caller without CAP_SYS_MODULE gets EPERM from
 * delete_module for any name and flags, and the parent still does after
 * the child has exited.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_delete_module 176
/* linux/capability.h */
#define CAP_VERSION_3 0x20080522
#define CAP_SYS_MODULE 16

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static __attribute__((unused)) i64 delete_module(const char *name, u64 flags) {
    return syscall2(SYS_delete_module, name, flags);
}

/* Empty all three capability sets (any user may do that; root then fails
 * every privilege check too) and confirm with capget that nothing is
 * left: 1 when the drop worked. */
static __attribute__((unused)) int drop_caps(void) {
    struct cap_header h = { CAP_VERSION_3, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return 0;
    h.version = CAP_VERSION_3;
    h.pid = 0;
    memset(d, 0xff, sizeof d);
    if (syscall2(SYS_capget, &h, d) != 0) return 0;
    return d[0].effective == 0 && d[1].effective == 0 && d[0].permitted == 0 && d[1].permitted == 0;
}

/* Set the effective and permitted sets of capabilities 0-31 (32-63 and
 * the inheritable set become empty); the capset result. */
static __attribute__((unused)) int set_caps(unsigned eff, unsigned prm) {
    struct cap_header h = { CAP_VERSION_3, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    d[0].effective = eff;
    d[0].permitted = prm;
    return (int)syscall2(SYS_capset, &h, d);
}

static __attribute__((unused)) void *map_pages(u64 n, int prot) {
    i64 r = syscall6(SYS_mmap, 0, n * 4096, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    return r < 0 ? (void *)0 : (void *)r;
}

/* Names no module on any system has. */
#define NO_SUCH_MODULE "poly_no_such_module_q7"
int test_main(void) {
    int dropped = drop_caps();
    check("caps_dropped", dropped, dropped);
    if (!dropped) return finish();

    i64 r = delete_module(NO_SUCH_MODULE, O_NONBLOCK);
    check("parent_before_eperm", r == -EPERM, r);
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        if (delete_module(NO_SUCH_MODULE, O_NONBLOCK) != -EPERM) code |= 1;
        if (delete_module(0, 0) != -EPERM) code |= 2;
        if (delete_module("", O_TRUNC) != -EPERM) code |= 4;
        exit_group(code);
    }
    check("fork", pid > 0, pid);
    int code = wait_exit(pid);
    check("child_exited", code >= 0, code);
    check("child_unknown_eperm", code >= 0 && !(code & 1), code);
    check("child_null_eperm", code >= 0 && !(code & 2), code);
    check("child_empty_eperm", code >= 0 && !(code & 4), code);
    r = delete_module(NO_SUCH_MODULE, O_NONBLOCK);
    check("parent_after_eperm", r == -EPERM, r);
    return finish();
}
