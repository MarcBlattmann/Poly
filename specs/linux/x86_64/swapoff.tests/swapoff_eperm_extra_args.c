/* swapoff takes one argument: whatever is in the other argument
 * registers does not change the result. Without CAP_SYS_ADMIN
 * (capabilities dropped) a regular file, a directory and a missing file
 * are EPERM (1) both with zeros and with -1 in registers 2 to 6. */
#include "poly_ipc.h"

#define SYS_swapoff 168

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

/* Empty all three capability sets: any user may do that, and root then
 * fails the privilege check like everybody else. */
static i64 drop_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

#define F "/tmp/swapoff_eperm_extra_file"

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0644));
    check("drop_caps", drop_caps() == 0, 0);

    i64 r = syscall6(SYS_swapoff, (i64)F, 0, 0, 0, 0, 0);
    check("file_zeros_eperm", r == -EPERM, r);
    r = syscall6(SYS_swapoff, (i64)F, -1, -1, -1, -1, -1);
    check("file_minus_one_eperm", r == -EPERM, r);
    r = syscall6(SYS_swapoff, (i64)"/tmp", 0x80000, 1, 2, 3, 4);
    check("dir_junk_eperm", r == -EPERM, r);
    r = syscall6(SYS_swapoff, (i64)"/tmp/swapoff_eperm_extra_missing", -1, 0, -1, 0, -1);
    check("missing_junk_eperm", r == -EPERM, r);

    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
