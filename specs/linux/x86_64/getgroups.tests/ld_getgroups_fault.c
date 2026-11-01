/* getgroups and bad list pointers, for whatever groups the caller has:
 * with a size of at least the count, a list that cannot take count gids
 * (NULL, a low address, a read-only page, a buffer that runs into an
 * unmapped page) is EFAULT when there is at least one group, and returns
 * 0 when there is none (nothing is written). With size 0 the pointer is
 * never used.
 */
#include "a_sys.h"

#define SYS_getgroups 115

int test_main(void) {
    i64 n = syscall2(SYS_getgroups, 0, 0);
    check("count_ok", n >= 0, n);
    i64 want = n > 0 ? -EFAULT : 0;
    i64 size = n > 0 ? n : 4;

    i64 r = syscall2(SYS_getgroups, size, 0);
    check("null_list", r == want, r);
    r = syscall2(SYS_getgroups, size, (void *)8);
    check("low_address_list", r == want, r);

    char *pages = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    r = syscall2(SYS_getgroups, size, pages);
    check("read_only_list", r == want, r);

    /* Writable first page, unmapped second page; the list starts 2 bytes
     * before the end of the first, so even one gid does not fit. */
    syscall3(SYS_mprotect, pages, A_PAGE, PROT_READ | PROT_WRITE);
    syscall2(SYS_munmap, pages + A_PAGE, A_PAGE);
    r = syscall2(SYS_getgroups, size, pages + A_PAGE - 2);
    check("crossing_into_unmapped", r == want, r);

    /* At the end of the writable page with room for exactly count gids. */
    r = syscall2(SYS_getgroups, size, pages + A_PAGE - 4 * n);
    check("exact_room_ok", r == n, r);

    r = syscall2(SYS_getgroups, 0, (void *)8);
    check("size_0_never_uses_list", r == n, r);
    return finish();
}
