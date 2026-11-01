/* pkey_alloc(flags, access_rights) on a CPU without protection keys
 * (CPUID leaf 7 ECX bit 4, OSPKE, clear; QEMU's default CPU and many
 * VMs): with valid arguments (flags 0, access_rights 0,
 * PKEY_DISABLE_ACCESS 1, PKEY_DISABLE_WRITE 2 or both) it fails with
 * ENOSPC (28), every time; no key is ever handed out, so pkey_free of
 * the keys 1..15 is EINVAL afterwards. On a CPU with OSPKE the test
 * prints a skip line and passes (the spec describes that case
 * separately).
 */
#include "a_sys.h"

#define SYS_pkey_alloc 330
#define SYS_pkey_free 331
#define ENOSPC 28
/* linux/mman.h (asm-generic/mman-common.h) */
#define PKEY_DISABLE_ACCESS 0x1
#define PKEY_DISABLE_WRITE 0x2

static int has_ospke(void) {
    u32 a, b, c, d;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(0), "c"(0));
    if (a < 7) return 0;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(7), "c"(0));
    return (c >> 4) & 1;
}

int test_main(void) {
    if (has_ospke()) {
        put("skip: this CPU has protection keys (OSPKE); the test covers CPUs without them\n");
        return 0;
    }
    expect("alloc_0_0", syscall2(SYS_pkey_alloc, 0, 0), -ENOSPC);
    expect("alloc_disable_access", syscall2(SYS_pkey_alloc, 0, PKEY_DISABLE_ACCESS), -ENOSPC);
    expect("alloc_disable_write", syscall2(SYS_pkey_alloc, 0, PKEY_DISABLE_WRITE), -ENOSPC);
    expect("alloc_both", syscall2(SYS_pkey_alloc, 0, PKEY_DISABLE_ACCESS | PKEY_DISABLE_WRITE), -ENOSPC);
    int same = 1;
    for (int i = 0; i < 20; i++)
        if (syscall2(SYS_pkey_alloc, 0, 0) != -ENOSPC) same = 0;
    check("alloc_always_enospc", same, 0);
    int none = 1;
    for (int k = 1; k < 16; k++)
        if (syscall1(SYS_pkey_free, k) != -EINVAL) none = 0;
    check("no_key_was_allocated", none, 0);
    /* No descriptor or other resource is consumed. */
    i64 fd = a_open("/dev/null", O_RDONLY, 0);
    expect("next_fd_is_3", fd, 3);
    a_close(fd);
    return finish();
}
