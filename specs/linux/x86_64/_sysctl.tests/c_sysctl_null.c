/* _sysctl with a NULL argument pointer: ENOSYS, not EFAULT (the call
 * does not exist, so nothing is read).
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
    expect("NULL args is ENOSYS", syscall1(SYS__sysctl, 0), -ENOSYS);
    expect("all-zero registers is ENOSYS", syscall6(SYS__sysctl, 0, 0, 0, 0, 0, 0), -ENOSYS);
    return finish();
}
