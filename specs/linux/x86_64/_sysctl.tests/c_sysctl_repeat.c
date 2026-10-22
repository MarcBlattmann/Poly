/* _sysctl fails the same way every time: 1000 calls in a row are all
 * ENOSYS, with no state built up between calls.
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
    char buf[32];
    size_t len = sizeof buf;
    struct k_sysctl_args a;
    memset(&a, 0, sizeof a);
    a.name = name;
    a.nlen = 2;
    a.oldval = buf;
    a.oldlenp = &len;
    int bad = 0;
    i64 last = 0;
    for (int i = 0; i < 1000; i++) {
        i64 r = syscall1(SYS__sysctl, i & 1 ? (i64)&a : 0);
        if (r != -ENOSYS) bad++, last = r;
    }
    check("1000 calls all ENOSYS", bad == 0, last);
    expect("length untouched", (i64)len, 32);
    return finish();
}
