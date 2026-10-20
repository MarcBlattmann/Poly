/* mmap / munmap / mprotect on anonymous memory, including the error cases. */
#include "poly_test.h"

#define PAGE 4096

static i64 map(i64 addr, i64 len, i64 prot, i64 flags) { return syscall6(SYS_mmap, addr, len, prot, flags, -1, 0); }

int test_main(void) {
    i64 a = map(0, 2 * PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS);
    check("mmap_anon_succeeds", a > 0, a < 0 ? a : 0);
    check("mmap_result_is_page_aligned", (a & (PAGE - 1)) == 0, a & (PAGE - 1));

    volatile unsigned char *p = (volatile unsigned char *)a;
    check("mmap_anon_is_zero", p[0] == 0 && p[2 * PAGE - 1] == 0, 0);
    p[0] = 0x5a;
    p[PAGE] = 0xa5;
    check("mmap_anon_is_writable", p[0] == 0x5a && p[PAGE] == 0xa5, 0);

    check("mprotect_read_only", syscall3(SYS_mprotect, a, PAGE, PROT_READ) == 0, 0);
    check("mprotect_read_only_still_readable", p[0] == 0x5a, p[0]);
    check("mprotect_unaligned_einval", syscall3(SYS_mprotect, a + 1, PAGE, PROT_READ) == -EINVAL, 0);

    check("munmap_succeeds", syscall2(SYS_munmap, a, 2 * PAGE) == 0, 0);
    check("munmap_twice_succeeds", syscall2(SYS_munmap, a, 2 * PAGE) == 0, 0);
    i64 r = syscall3(SYS_mprotect, a, PAGE, PROT_READ);
    check("mprotect_unmapped_enomem", r == -ENOMEM, r);

    r = syscall2(SYS_munmap, a + 1, PAGE);
    check("munmap_unaligned_einval", r == -EINVAL, r);
    r = map(0, 0, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS);
    check("mmap_zero_length_einval", r == -EINVAL, r);
    r = map(0, PAGE, PROT_READ, MAP_ANONYMOUS);
    check("mmap_neither_shared_nor_private_einval", r == -EINVAL, r);
    r = map(0, PAGE, PROT_READ, MAP_PRIVATE | MAP_SHARED | MAP_ANONYMOUS);
    check("mmap_shared_and_private_einval", r == -EINVAL, r);
    r = map(1, PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED);
    check("mmap_fixed_unaligned_einval", r == -EINVAL, r);
    r = syscall6(SYS_mmap, 0, PAGE, PROT_READ, MAP_PRIVATE, 1000, 0);
    check("mmap_file_bad_fd_ebadf", r == -EBADF, r);

    /* A hint is only a hint: MAP_FIXED puts the mapping exactly there. */
    i64 b = map(0, 4 * PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS);
    i64 c = map(b + PAGE, PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED);
    check("mmap_fixed_lands_at_address", c == b + PAGE, c - b);
    check("munmap_range_succeeds", syscall2(SYS_munmap, b, 4 * PAGE) == 0, 0);
    return finish();
}
