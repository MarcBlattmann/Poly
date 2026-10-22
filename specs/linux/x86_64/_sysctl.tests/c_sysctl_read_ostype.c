/* _sysctl was removed from Linux (5.5): reading kern.ostype, the man
 * page example, fails with ENOSYS. Nothing is written back: the old
 * value buffer and the length word keep what the caller put there.
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
    char buf[100];
    for (int i = 0; i < 100; i++) buf[i] = 'X';
    size_t len = sizeof buf;
    struct k_sysctl_args a;
    memset(&a, 0, sizeof a);
    a.name = name;
    a.nlen = 2;
    a.oldval = buf;
    a.oldlenp = &len;
    expect("read kern.ostype fails with ENOSYS", syscall1(SYS__sysctl, &a), -ENOSYS);
    expect("oldlen untouched", (i64)len, 100);
    int same = 1;
    for (int i = 0; i < 100; i++) same &= buf[i] == 'X';
    check("old value buffer untouched", same, buf[0]);
    expect("name vector untouched", name[0] * 10 + name[1], 11);
    expect("nlen untouched", a.nlen, 2);
    return finish();
}
