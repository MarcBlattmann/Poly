/* Option 2 with a valid index and a buffer it cannot write is EFAULT:
 * NULL, a low address, a kernel address, a read-only page, and a buffer
 * whose name would run into an inaccessible page. (Bytes before the
 * fault may or may not have been written; not checked.) Afterwards the
 * same call with a good buffer still works.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

int test_main(void) {
    i64 p = sysfs3(1, (i64)"proc", 0);
    i64 r = sysfs3(2, p, 0);
    check("null_efault", r == -EFAULT, r);
    r = sysfs3(2, p, 8);
    check("low_efault", r == -EFAULT, r);
    r = sysfs3(2, p, (i64)0xffffffff81000000UL);
    check("kernel_efault", r == -EFAULT, r);

    char *m = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    r = syscall3(SYS_mprotect, m + 4096, 4096, PROT_NONE);
    check("guard_ok", r == 0, r);
    r = sysfs3(2, p, (i64)(m + 4096));
    check("guard_page_efault", r == -EFAULT, r);
    r = sysfs3(2, p, (i64)(m + 4096 - 1));
    check("straddle_1_efault", r == -EFAULT, r);
    r = sysfs3(2, p, (i64)(m + 4096 - 4));
    check("straddle_nul_efault", r == -EFAULT, r);
    r = sysfs3(2, p, (i64)(m + 4096 - 5));
    check("exact_fit_ok", r == 0, r);

    r = syscall3(SYS_mprotect, m, 4096, PROT_READ);
    check("ro_ok", r == 0, r);
    r = sysfs3(2, p, (i64)m);
    check("ro_page_efault", r == -EFAULT, r);
    r = sysfs3(2, p, (i64)"string literal");
    check("rodata_efault", r == -EFAULT, r);

    char buf[32];
    memset(buf, 0, sizeof buf);
    r = sysfs3(2, p, (i64)buf);
    check("good_after", r == 0 && str_eq(buf, "proc"), r);
    return finish();
}
