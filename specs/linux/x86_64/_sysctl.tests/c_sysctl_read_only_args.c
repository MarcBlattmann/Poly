/* The argument block, name vector and length word may all be in
 * read-only memory: _sysctl fails with ENOSYS, never EFAULT, and no
 * SIGSEGV arrives (nothing is written).
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
    char *p = (char *)syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)p > 0, (i64)p);
    if ((i64)p <= 0) return finish();
    struct k_sysctl_args *a = (struct k_sysctl_args *)p;
    int *name = (int *)(p + 512);
    size_t *len = (size_t *)(p + 1024);
    name[0] = CTL_KERN;
    name[1] = KERN_OSRELEASE;
    *len = 64;
    a->name = name;
    a->nlen = 2;
    a->oldval = p + 2048;
    a->oldlenp = len;
    expect("mprotect read-only", syscall3(SYS_mprotect, p, 4096, PROT_READ), 0);
    expect("read-only args is ENOSYS", syscall1(SYS__sysctl, a), -ENOSYS);
    expect("length still 64", (i64)*len, 64);
    expect("old value area still zero", p[2048], 0);
    expect("PROT_NONE args", syscall3(SYS_mprotect, p, 4096, PROT_NONE), 0);
    expect("unreadable args is ENOSYS", syscall1(SYS__sysctl, p), -ENOSYS);
    return finish();
}
