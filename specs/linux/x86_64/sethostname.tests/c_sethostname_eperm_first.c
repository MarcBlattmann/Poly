/* The permission check comes before every argument check: without
 * CAP_SYS_ADMIN (capability sets emptied first, so root and normal users
 * see the same), a length over 64, a negative length, a NULL or unmapped
 * name and even a zero length all fail with EPERM, never EINVAL or
 * EFAULT, and the host name stays as it was.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_capget 125
#define SYS_capset 126
#define SYS_sethostname 170

/* linux/capability.h: _LINUX_CAPABILITY_VERSION_3, two data records */
struct cap_header {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

static int drop_all_caps(void) {
    struct cap_header h = { 0x20080522, 0 };
    struct cap_data d[2], e[2];
    memset(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return 0;
    memset(e, 0xff, sizeof e);
    if (syscall2(SYS_capget, &h, e) != 0) return 0;
    return e[0].effective == 0 && e[1].effective == 0 && e[0].permitted == 0 && e[1].permitted == 0;
}

static char big[4096];

int test_main(void) {
    struct a_utsname before, after;
    memset(&before, 0, sizeof before);
    memset(&after, 0, sizeof after);
    memset(big, 'h', sizeof big);
    syscall1(SYS_uname, &before);

    int dropped = drop_all_caps();
    check("caps_dropped", dropped, 0);
    if (!dropped) return finish(); /* never try to rename the real host */

    /* An address with nothing mapped: a page mapped and unmapped again. */
    i64 gone = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, gone, A_PAGE);

    expect("len_65_eperm", syscall2(SYS_sethostname, big, 65), -EPERM);
    expect("len_4096_eperm", syscall2(SYS_sethostname, big, 4096), -EPERM);
    expect("len_minus_1_eperm", syscall2(SYS_sethostname, big, -1), -EPERM);
    expect("len_int_min_eperm", syscall2(SYS_sethostname, big, 0x80000000L), -EPERM);
    expect("len_zero_eperm", syscall2(SYS_sethostname, big, 0), -EPERM);
    expect("null_name_eperm", syscall2(SYS_sethostname, 0, 5), -EPERM);
    expect("null_len_zero_eperm", syscall2(SYS_sethostname, 0, 0), -EPERM);
    expect("unmapped_name_eperm", syscall2(SYS_sethostname, gone, 8), -EPERM);
    expect("null_len_65_eperm", syscall2(SYS_sethostname, 0, 65), -EPERM);

    syscall1(SYS_uname, &after);
    check("nodename_unchanged", mem_eq(before.nodename, after.nodename, sizeof before.nodename), 0);
    return finish();
}
