/* The privilege check comes before the name is read: a name pointer that
 * is NULL, 1, a low address, an unmapped page, a PROT_NONE page, a string
 * that runs into an unmapped page without a NUL, a non-canonical address,
 * a kernel address or the top page of the address space all give EPERM
 * (not EFAULT) without CAP_SYS_MODULE, and no signal is raised.
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

    i64 r = delete_module(0, O_NONBLOCK);
    check("null_eperm", r == -EPERM, r);
    r = delete_module((const char *)1, O_NONBLOCK);
    check("one_eperm", r == -EPERM, r);
    r = delete_module((const char *)0x1000, 0);
    check("low_address_eperm", r == -EPERM, r);

    char *p = map_pages(2, PROT_READ | PROT_WRITE);
    check("mmap", p != 0, 0);
    if (!p) return finish();
    memset(p, 'a', 4096);
    syscall2(SYS_munmap, p + 4096, 4096);
    r = delete_module(p + 4096, O_NONBLOCK);
    check("unmapped_eperm", r == -EPERM, r);
    r = delete_module(p + 4096 - 3, O_NONBLOCK);
    check("runs_into_unmapped_eperm", r == -EPERM, r);
    syscall3(SYS_mprotect, p, 4096, PROT_NONE);
    r = delete_module(p, O_NONBLOCK);
    check("prot_none_eperm", r == -EPERM, r);
    r = delete_module((const char *)0x8000000000000000UL, O_NONBLOCK);
    check("non_canonical_eperm", r == -EPERM, r);
    r = delete_module((const char *)0xffffffff81000000UL, O_NONBLOCK);
    check("kernel_address_eperm", r == -EPERM, r);
    r = delete_module((const char *)0xfffffffffffff000UL, O_TRUNC);
    check("top_of_address_space_eperm", r == -EPERM, r);
    return finish();
}
