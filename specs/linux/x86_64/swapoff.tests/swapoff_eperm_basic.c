/* swapoff needs CAP_SYS_ADMIN in the effective set. After emptying every
 * capability set with capset (any user may, so root fails the check
 * too), swapoff of an existing regular file, a directory, /dev/null and
 * a missing file are all EPERM (1). */
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

#define F "/tmp/swapoff_eperm_basic_file"

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0644));
    check("drop_caps", drop_caps() == 0, 0);

    i64 r = syscall1(SYS_swapoff, F);
    check("file_eperm", r == -EPERM, r);
    r = syscall1(SYS_swapoff, "/tmp");
    check("dir_eperm", r == -EPERM, r);
    r = syscall1(SYS_swapoff, "/dev/null");
    check("devnull_eperm", r == -EPERM, r);
    r = syscall1(SYS_swapoff, "/tmp/swapoff_eperm_basic_missing");
    check("missing_eperm", r == -EPERM, r);
    r = syscall1(SYS_swapoff, F);
    check("again_eperm", r == -EPERM, r);

    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
