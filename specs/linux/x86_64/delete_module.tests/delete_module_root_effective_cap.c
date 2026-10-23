/* As root, only the effective set decides: with CAP_SYS_MODULE removed
 * from the effective set (still permitted) delete_module is EPERM, before
 * the name is read; raising it again gives ENOENT for an unknown name and
 * EFAULT for a NULL one; once it is removed from the permitted set too it
 * cannot be raised again and delete_module stays EPERM. Runs in a child.
 * poly-diff: needs=root (needs CAP_SYS_MODULE to start with)
 * poly-diff: syscalls=off (a child process)
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
    i64 r = delete_module(NO_SUCH_MODULE, O_NONBLOCK);
    check("root_enoent", r == -ENOENT, r);
    if (r == -EPERM) return finish();
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        unsigned all = 0xffffffffU, mod = 1U << CAP_SYS_MODULE;
        if (set_caps(all & ~mod, all) != 0) code |= 1;
        if (delete_module(NO_SUCH_MODULE, O_NONBLOCK) != -EPERM) code |= 2;
        if (delete_module(0, O_NONBLOCK) != -EPERM) code |= 2;
        if (set_caps(all, all) != 0) code |= 4;
        if (delete_module(NO_SUCH_MODULE, O_NONBLOCK) != -ENOENT) code |= 8;
        if (delete_module(0, O_NONBLOCK) != -EFAULT) code |= 8;
        if (set_caps(all & ~mod, all & ~mod) != 0) code |= 16;
        if (set_caps(all, all & ~mod) != -EPERM) code |= 32;
        if (delete_module(NO_SUCH_MODULE, O_NONBLOCK) != -EPERM) code |= 64;
        exit_group(code);
    }
    int code = wait_exit(pid);
    check("child_exited", code >= 0, code);
    check("drop_effective_ok", code >= 0 && !(code & 1), code);
    check("effective_dropped_eperm", code >= 0 && !(code & 2), code);
    check("raise_again_ok", code >= 0 && !(code & 4), code);
    check("raised_enoent_efault", code >= 0 && !(code & 8), code);
    check("drop_permitted_ok", code >= 0 && !(code & 16), code);
    check("cannot_raise_again", code >= 0 && !(code & 32), code);
    check("permitted_dropped_eperm", code >= 0 && !(code & 64), code);
    return finish();
}
