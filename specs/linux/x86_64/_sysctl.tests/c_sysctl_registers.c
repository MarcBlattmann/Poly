/* Like every system call, a failing _sysctl leaves the argument
 * registers (rdi, rsi, rdx, r10, r8, r9) as they were; only rax (the
 * result) changes, plus rcx and r11, which the syscall instruction uses.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS__sysctl 156
/* linux/sysctl.h */
#define CTL_KERN 1
#define CTL_VM 2
#define KERN_OSTYPE 1
#define KERN_OSRELEASE 2
#define KERN_NODENAME 7
#define KERN_DOMAINNAME 8

/* struct __sysctl_args (linux/sysctl.h), 80 bytes */
struct k_sysctl_args {
    int *name;
    int nlen;
    void *oldval;
    size_t *oldlenp;
    void *newval;
    size_t newlen;
    unsigned long unused[4];
};

int test_main(void) {
    register i64 rdi __asm__("rdi") = 0x1111111111111111;
    register i64 rsi __asm__("rsi") = 0x2222222222222222;
    register i64 rdx __asm__("rdx") = 0x3333333333333333;
    register i64 r10 __asm__("r10") = 0x4444444444444444;
    register i64 r8 __asm__("r8") = 0x5555555555555555;
    register i64 r9 __asm__("r9") = 0x6666666666666666;
    i64 rax = SYS__sysctl;
    __asm__ volatile("syscall"
                     : "+a"(rax), "+r"(rdi), "+r"(rsi), "+r"(rdx), "+r"(r10), "+r"(r8), "+r"(r9)
                     :
                     : "rcx", "r11", "memory");
    i64 v[6] = {rdi, rsi, rdx, r10, r8, r9};
    expect("result ENOSYS", rax, -ENOSYS);
    expect("rdi kept", v[0] == 0x1111111111111111, 1);
    expect("rsi kept", v[1] == 0x2222222222222222, 1);
    expect("rdx kept", v[2] == 0x3333333333333333, 1);
    expect("r10 kept", v[3] == 0x4444444444444444, 1);
    expect("r8 kept", v[4] == 0x5555555555555555, 1);
    expect("r9 kept", v[5] == 0x6666666666666666, 1);
    return finish();
}
