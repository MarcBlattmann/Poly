/* Option 2 writes the name and its NUL and nothing else: the bytes
 * before the buffer and after the NUL keep their values. A buffer that
 * ends exactly at the NUL, right before an inaccessible page, works.
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

int test_main(void) {
    i64 p = sysfs3(1, (i64)"proc", 0);
    i64 t = sysfs3(1, (i64)"tmpfs", 0);
    char buf[64];
    memset(buf, 'X', sizeof buf);
    i64 r = sysfs3(2, p, (i64)(buf + 8));
    check("proc_ok", r == 0, r);
    check("proc_name", mem_eq(buf + 8, "proc", 5), r);
    check("before_untouched", mem_eq(buf, "XXXXXXXX", 8), buf[7]);
    check("after_untouched", buf[13] == 'X' && buf[63] == 'X', buf[13]);
    /* A shorter name over a longer one leaves the rest. */
    memset(buf, 'X', sizeof buf);
    sysfs3(2, t, (i64)buf);
    sysfs3(2, p, (i64)buf);
    check("overwrite_prefix", mem_eq(buf, "proc\0\0X", 7), buf[5]);

    char *m = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", (i64)m > 0, (i64)m);
    if ((i64)m <= 0) return finish();
    r = syscall3(SYS_mprotect, m + 4096, 4096, PROT_NONE);
    check("guard_ok", r == 0, r);
    memset(m, 'Q', 4096);
    r = sysfs3(2, t, (i64)(m + 4096 - 6));
    check("exact_fit_ok", r == 0, r);
    check("exact_fit_name", mem_eq(m + 4090, "tmpfs", 6), r);
    check("exact_fit_before", m[4089] == 'Q', m[4089]);
    return finish();
}
