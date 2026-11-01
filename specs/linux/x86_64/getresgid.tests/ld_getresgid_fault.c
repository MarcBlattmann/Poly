/* getresgid with unwritable pointers is EFAULT. The ids are written in the
 * order real, effective, saved, each only if the earlier writes worked:
 * with a bad effective pointer the real id has been written and the saved
 * id not; with a bad saved pointer the real and effective ids have been
 * written; with a bad real pointer nothing is written. NULL, a low address
 * and a read-only page all fail.
 */
#include "a_sys.h"

#define SYS_getresgid 120

static const u32 read_only = 0x12345678;

int test_main(void) {
    u32 rg = syscall0(SYS_getgid), eg = syscall0(SYS_getegid);
    u32 a, b, c;
    i64 r;

    r = syscall3(SYS_getresgid, 0, 0, 0);
    check("all_null_efault", r == -EFAULT, r);

    a = b = c = 0xdead;
    r = syscall3(SYS_getresgid, 0, &b, &c);
    check("real_null_efault", r == -EFAULT, r);
    check("real_null_wrote_nothing", b == 0xdead && c == 0xdead, 0);

    a = b = c = 0xdead;
    r = syscall3(SYS_getresgid, &a, 0, &c);
    check("effective_null_efault", r == -EFAULT, r);
    check("effective_null_wrote_real", a == rg, a);
    check("effective_null_not_saved", c == 0xdead, c);

    a = b = c = 0xdead;
    r = syscall3(SYS_getresgid, &a, &b, 0);
    check("saved_null_efault", r == -EFAULT, r);
    check("saved_null_wrote_real", a == rg, a);
    check("saved_null_wrote_effective", b == eg, b);

    a = b = c = 0xdead;
    r = syscall3(SYS_getresgid, &a, &b, (void *)16);
    check("low_address_efault", r == -EFAULT, r);

    r = syscall3(SYS_getresgid, (void *)&read_only, &b, &c);
    check("read_only_efault", r == -EFAULT, r);
    check("read_only_unchanged", read_only == 0x12345678, 0);

    void *page = (void *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    a = 0xdead;
    r = syscall3(SYS_getresgid, &a, page, &c);
    check("prot_read_page_efault", r == -EFAULT, r);
    check("prot_read_page_real_written", a == rg, a);
    syscall2(SYS_munmap, page, A_PAGE);
    r = syscall3(SYS_getresgid, &a, &b, page);
    check("unmapped_efault", r == -EFAULT, r);
    return finish();
}
