/* Option 1 reads at most 4096 bytes of the name. A name with no NUL in
 * its first 4096 bytes is EINVAL even when the next byte is
 * unreadable; one that hits an unreadable byte within its first 4096
 * bytes is EFAULT. Long names (4095, 4096, 8191 bytes) are EINVAL, not
 * ENAMETOOLONG.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

static char big[8192];

int test_main(void) {
    memset(big, 'a', sizeof big);
    big[4095] = 0;
    i64 r = sysfs3(1, (i64)big, 0);
    check("len_4095_einval", r == -EINVAL, r);
    big[4095] = 'a';
    big[4096] = 0;
    r = sysfs3(1, (i64)big, 0);
    check("len_4096_einval", r == -EINVAL, r);
    big[4096] = 'a';
    big[8191] = 0;
    r = sysfs3(1, (i64)big, 0);
    check("len_8191_einval", r == -EINVAL, r);

    char *m = (char *)syscall6(SYS_mmap, 0, 3 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    r = syscall3(SYS_mprotect, m + 8192, 4096, PROT_NONE);
    check("guard_ok", r == 0, r);
    memset(m, 'a', 8192);
    char *end = m + 8192;
    r = sysfs3(1, (i64)(end - 4100), 0);
    check("no_nul_4100_einval", r == -EINVAL, r);
    r = sysfs3(1, (i64)(end - 4096), 0);
    check("no_nul_4096_einval", r == -EINVAL, r);
    r = sysfs3(1, (i64)(end - 4095), 0);
    check("no_nul_4095_efault", r == -EFAULT, r);
    r = sysfs3(1, (i64)(end - 100), 0);
    check("no_nul_100_efault", r == -EFAULT, r);
    r = sysfs3(1, (i64)(end - 1), 0);
    check("no_nul_1_efault", r == -EFAULT, r);
    return finish();
}
