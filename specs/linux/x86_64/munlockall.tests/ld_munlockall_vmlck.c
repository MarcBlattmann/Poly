/* munlockall unlocks everything: the "VmLck:" line of /proc/self/status
 * is 0 kB afterwards, after mlock2 of several ranges and after
 * mlockall(MCL_CURRENT) (VmLck above 0 before). A mapping made after it
 * adds nothing to VmLck.
 * poly-diff: syscalls=names (the read of /proc/self/status returns text
 * with sizes and ids of the process)
 */
#include "a_sys.h"

#define SYS_mlock2 325
#define MLOCK_ONFAULT 1
#define MCL_CURRENT 1
#define MCL_FUTURE 2

static char status[8192];
static i64 vmlck(void) {
    i64 fd = a_open("/proc/self/status", O_RDONLY, 0);
    if (fd < 0) return fd;
    i64 n = 0, r;
    while (n < (i64)sizeof status - 1 && (r = a_read(fd, status + n, sizeof status - 1 - n)) > 0) n += r;
    a_close(fd);
    status[n] = 0;
    for (i64 i = 0; i + 6 < n; i++)
        if (mem_eq(status + i, "\nVmLck:", 7)) {
            char *p = status + i + 7;
            while (*p == ' ' || *p == '\t') p++;
            return (i64)a_atou(p);
        }
    return -1;
}
static char *map(int pages) {
    return (char *)syscall6(SYS_mmap, 0, (u64)pages * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1,
                            0);
}

int test_main(void) {
    char *a = map(4), *b = map(4);
    expect("start_0", vmlck(), 0);
    expect("lock_a", syscall3(SYS_mlock2, a, 2 * A_PAGE, 0), 0);
    expect("lock_b", syscall3(SYS_mlock2, b, 3 * A_PAGE, MLOCK_ONFAULT), 0);
    expect("vmlck_20", vmlck(), 20);
    expect("munlockall", syscall0(SYS_munlockall), 0);
    expect("vmlck_0", vmlck(), 0);

    expect("mlockall_current_future", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    check("vmlck_positive", vmlck() > 0, 0);
    expect("munlockall_2", syscall0(SYS_munlockall), 0);
    expect("vmlck_0_again", vmlck(), 0);
    char *c = map(4);
    c[0] = 1;
    expect("new_mapping_not_locked", vmlck(), 0);
    return finish();
}
