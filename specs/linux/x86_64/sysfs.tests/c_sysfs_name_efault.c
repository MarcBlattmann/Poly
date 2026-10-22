/* Option 1 with an unreadable name is EFAULT: NULL, a low address, a
 * kernel address, and a name that runs into an inaccessible page before
 * its NUL. A name whose NUL is the last readable byte is fine. A bad
 * option is decided before the name is read (EINVAL with a NULL name).
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

int test_main(void) {
    i64 r = sysfs3(1, 0, 0);
    check("null_efault", r == -EFAULT, r);
    r = sysfs3(1, 1, 0);
    check("low_efault", r == -EFAULT, r);
    r = sysfs3(1, (i64)0xffffffff81000000UL, 0);
    check("kernel_efault", r == -EFAULT, r);
    r = sysfs3(1, (i64)0x800000000000UL, 0);
    check("noncanonical_efault", r == -EFAULT, r);
    r = sysfs3(0, 0, 0);
    check("bad_option_before_name", r == -EINVAL, r);

    char *m = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    r = syscall3(SYS_mprotect, m + 4096, 4096, PROT_NONE);
    check("guard_ok", r == 0, r);
    i64 want = sysfs3(1, (i64)"proc", 0);
    memcpy(m + 4092, "proc", 4);
    r = sysfs3(1, (i64)(m + 4092), 0);
    check("unterminated_efault", r == -EFAULT, r);
    memcpy(m + 4091, "proc", 5);
    r = sysfs3(1, (i64)(m + 4091), 0);
    check("nul_last_byte_ok", r == want, r);
    r = sysfs3(1, (i64)(m + 4096), 0);
    check("guard_page_efault", r == -EFAULT, r);
    m[4095] = 0;
    r = sysfs3(1, (i64)(m + 4095), 0);
    check("empty_last_byte_einval", r == -EINVAL, r);
    /* A read-only page is readable. */
    memcpy(m, "tmpfs", 6);
    r = syscall3(SYS_mprotect, m, 4096, PROT_READ);
    check("ro_ok", r == 0, r);
    r = sysfs3(1, (i64)m, 0);
    check("ro_name_ok", r == sysfs3(1, (i64)"tmpfs", 0) && r >= 0, r);
    return finish();
}
