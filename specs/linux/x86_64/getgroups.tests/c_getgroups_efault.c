/* With a size large enough, a buffer the kernel cannot write is EFAULT
 * when there is at least one group to copy; with no groups nothing is
 * copied and the call returns 0. Bad buffers: NULL, a low address, an
 * unmapped page, a read-only page, a kernel address.
 */
#include "poly_ipc.h"

static i64 map(int prot) { return syscall6(SYS_mmap, 0, 4096, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0); }

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    i64 want = n > 0 ? -EFAULT : 0;
    i64 r = syscall2(SYS_getgroups, 65536, 0);
    check("null_buffer", r == want, r);
    r = syscall2(SYS_getgroups, 65536, 8);
    check("low_address", r == want, r);
    i64 ro = map(PROT_READ);
    r = syscall2(SYS_getgroups, 1024, ro);
    check("read_only_page", r == want, r);
    i64 none = map(PROT_NONE);
    r = syscall2(SYS_getgroups, 1024, none);
    check("prot_none_page", r == want, r);
    i64 gone = map(PROT_READ | PROT_WRITE);
    syscall2(SYS_munmap, gone, 4096);
    r = syscall2(SYS_getgroups, 1024, gone);
    check("unmapped_page", r == want, r);
    r = syscall2(SYS_getgroups, 1024, (i64)0xffff800000000000UL);
    check("kernel_address", r == want, r);
    /* A good buffer still works afterwards. */
    unsigned buf[1024];
    r = syscall2(SYS_getgroups, n <= 1024 ? 1024 : n, n <= 1024 ? buf : 0);
    check("good_buffer_after", n > 1024 || r == n, r);
    return finish();
}
