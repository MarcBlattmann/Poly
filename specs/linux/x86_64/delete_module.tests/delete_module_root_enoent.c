/* With CAP_SYS_MODULE, a readable name that is not a loaded module is
 * ENOENT (2), whatever the flags: 0, O_NONBLOCK, O_TRUNC, both, every bit
 * set; the flags are not checked before the name is looked up. The empty
 * name and a one-letter name are ENOENT too. Only names no module has are
 * used, so nothing can ever be unloaded.
 * poly-diff: needs=root (only a caller with CAP_SYS_MODULE gets past the privilege check)
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
    check("unknown_nonblock_enoent", r == -ENOENT, r);
    if (r == -EPERM) return finish();
    r = delete_module(NO_SUCH_MODULE, 0);
    check("unknown_zero_flags_enoent", r == -ENOENT, r);
    r = delete_module(NO_SUCH_MODULE, O_TRUNC);
    check("unknown_trunc_enoent", r == -ENOENT, r);
    r = delete_module(NO_SUCH_MODULE, O_NONBLOCK | O_TRUNC);
    check("unknown_force_nonblock_enoent", r == -ENOENT, r);
    r = delete_module(NO_SUCH_MODULE, 0xffffffffUL);
    check("unknown_all_flags_enoent", r == -ENOENT, r);
    r = delete_module("", O_NONBLOCK);
    check("empty_name_enoent", r == -ENOENT, r);
    r = delete_module("q", O_NONBLOCK);
    check("one_letter_enoent", r == -ENOENT, r);
    r = delete_module("poly no/such\tmodule", O_NONBLOCK);
    check("odd_characters_enoent", r == -ENOENT, r);
    r = delete_module(NO_SUCH_MODULE, O_NONBLOCK);
    check("again_enoent", r == -ENOENT, r);
    return finish();
}
