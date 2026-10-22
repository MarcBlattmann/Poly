/* Whatever nlen says (0, negative, 1, CTL_MAXNAME 10, above it, huge),
 * _sysctl fails with ENOSYS: there is no EINVAL or ENOTDIR path left.
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
    int name[16];
    for (int i = 0; i < 16; i++) name[i] = 1;
    char buf[64];
    size_t len = sizeof buf;
    struct k_sysctl_args a;
    memset(&a, 0, sizeof a);
    a.name = name;
    a.oldval = buf;
    a.oldlenp = &len;
    static const int lens[] = {0, -1, 1, 2, 10, 11, 16, 0x7fffffff, (int)0x80000000};
    static const char *const labels[] = {"nlen 0", "nlen -1", "nlen 1", "nlen 2", "nlen 10",
                                         "nlen 11", "nlen 16", "nlen INT_MAX", "nlen INT_MIN"};
    for (int i = 0; i < 9; i++) {
        a.nlen = lens[i];
        expect(labels[i], syscall1(SYS__sysctl, &a), -ENOSYS);
    }
    expect("length untouched", (i64)len, 64);
    return finish();
}
