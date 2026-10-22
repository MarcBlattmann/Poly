/* Without CAP_SYS_ADMIN swapon is EPERM (1) for every target: a missing
 * file, a directory, an empty regular file, a character device and a NULL
 * path, with flags 0 and with valid flags. Every capability is dropped
 * with capset first, so root and a normal user behave the same. */
#include "poly_ipc.h"

#define SYS_swapon 167
#define SWAP_FLAG_PREFER 0x8000  /* sys/swap.h */
#define SWAP_FLAG_DISCARD 0x10000 /* sys/swap.h */

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

#define F "/tmp/swapon_eperm_basic_file"

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0644));
    check("drop_caps", drop_caps() == 0, 0);

    i64 r = syscall2(SYS_swapon, "/tmp/swapon_eperm_basic_missing", 0);
    check("missing_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, "/tmp", 0);
    check("directory_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, F, 0);
    check("regular_file_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, "/dev/null", 0);
    check("char_device_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, 0, 0);
    check("null_path_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, F, SWAP_FLAG_PREFER | 10);
    check("prefer_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, F, SWAP_FLAG_DISCARD);
    check("discard_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, "/", 0);
    check("root_dir_eperm", r == -EPERM, r);

    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
