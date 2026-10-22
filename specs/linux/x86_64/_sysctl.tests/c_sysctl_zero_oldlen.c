/* The man page lists EFAULT for an oldval with zero room in *oldlenp;
 * on a kernel without the call that is ENOSYS too, as is a NULL oldlenp
 * with a non-NULL oldval and a newval with a huge newlen.
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
    int name[2] = {CTL_KERN, KERN_OSTYPE};
    char buf[64];
    size_t len = 0;
    struct k_sysctl_args a;
    memset(&a, 0, sizeof a);
    a.name = name;
    a.nlen = 2;
    a.oldval = buf;
    a.oldlenp = &len;
    expect("oldlen 0 is ENOSYS", syscall1(SYS__sysctl, &a), -ENOSYS);
    expect("oldlen stays 0", (i64)len, 0);
    a.oldlenp = 0;
    expect("NULL oldlenp is ENOSYS", syscall1(SYS__sysctl, &a), -ENOSYS);
    a.oldval = 0;
    a.newval = buf;
    a.newlen = (size_t)-1;
    expect("huge newlen is ENOSYS", syscall1(SYS__sysctl, &a), -ENOSYS);
    return finish();
}
