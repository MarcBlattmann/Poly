/* Names that never existed (top level 0, 9999, negative) and ones that
 * once did (vm, kern.osrelease) are treated alike: ENOSYS, not ENOTDIR.
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

static i64 try_name(int n0, int n1) {
    int name[2] = {n0, n1};
    char buf[64];
    size_t len = sizeof buf;
    struct k_sysctl_args a;
    memset(&a, 0, sizeof a);
    a.name = name;
    a.nlen = 2;
    a.oldval = buf;
    a.oldlenp = &len;
    return syscall1(SYS__sysctl, &a);
}

int test_main(void) {
    expect("name {0,0}", try_name(0, 0), -ENOSYS);
    expect("name {9999,1}", try_name(9999, 1), -ENOSYS);
    expect("name {-1,-1}", try_name(-1, -1), -ENOSYS);
    expect("name {CTL_VM,1}", try_name(CTL_VM, 1), -ENOSYS);
    expect("name {CTL_KERN,KERN_OSRELEASE}", try_name(CTL_KERN, KERN_OSRELEASE), -ENOSYS);
    return finish();
}
