/* pkey_free of the default key 0 without protection keys: EINVAL (22),
 * the same on every attempt, and it changes nothing: memory stays
 * readable and writable, mprotect and pkey_mprotect(-1) work, pkey 0 is
 * still refused by pkey_mprotect (EINVAL). pkey_free of a key after a
 * failed pkey_alloc (ENOSPC 28) is EINVAL too. On a CPU with OSPKE the
 * test prints a skip line and passes.
 */
#include "a_sys.h"

#define SYS_pkey_mprotect 329
#define SYS_pkey_alloc 330
#define SYS_pkey_free 331
#define ENOSPC 28

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
    char *m = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    m[0] = 1;
    expect("free_0", syscall1(SYS_pkey_free, 0), -EINVAL);
    expect("free_0_again", syscall1(SYS_pkey_free, 0), -EINVAL);
    m[0] = 2;
    check("memory_still_writable", m[0] == 2, m[0]);
    expect("pkey_mprotect_0_refused", syscall4(SYS_pkey_mprotect, m, A_PAGE, PROT_READ | PROT_WRITE, 0), -EINVAL);
    expect("pkey_mprotect_minus1", syscall4(SYS_pkey_mprotect, m, A_PAGE, PROT_READ | PROT_WRITE, -1), 0);
    expect("mprotect", syscall3(SYS_mprotect, m, A_PAGE, PROT_READ | PROT_WRITE), 0);
    m[0] = 3;
    check("writable_after", m[0] == 3, m[0]);

    i64 k = syscall2(SYS_pkey_alloc, 0, 0);
    expect("alloc_enospc", k, -ENOSPC);
    expect("free_1_after_failed_alloc", syscall1(SYS_pkey_free, 1), -EINVAL);
    expect("free_the_error_value", syscall1(SYS_pkey_free, k), -EINVAL);
    return finish();
}
