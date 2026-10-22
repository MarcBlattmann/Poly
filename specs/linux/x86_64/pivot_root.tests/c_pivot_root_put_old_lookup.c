/* pivot_root resolves put_old after new_root and before the privilege
 * check: with all capabilities dropped and new_root an existing
 * directory, a put_old that does not exist is ENOENT, an empty string
 * ENOENT, a regular file ENOTDIR (also "file/"), NULL or a bad pointer
 * EFAULT; only when both resolve to directories is it EPERM. */
#include "poly_ipc.h"

#define SYS_pivot_root 155
#define SYS_getcwd 79
#define SYS_chdir 80
#define SYS_fchmodat 268

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

/* Empty all three capability sets: allowed for any user, and afterwards
 * root is as unprivileged as a normal user. */
static i64 drop_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

static __attribute__((unused)) void make_file(const char *path) {
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY, 0644));
}
#define D "/tmp/c_pivot_root_pol"

int test_main(void) {
    syscall1(SYS_chdir, "/");
    check("drop_caps", drop_caps() == 0, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/d", AT_REMOVEDIR);
    syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, D "/d", 0755);
    make_file(D "/f");

    i64 r = syscall2(SYS_pivot_root, D, D "/missing");
    check("missing_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, D, "");
    check("empty_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, D, D "/f");
    check("file_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, D, D "/f/");
    check("file_slash_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, D, 0);
    check("null_efault", r == -EFAULT, r);
    r = syscall2(SYS_pivot_root, D, 8);
    check("bad_pointer_efault", r == -EFAULT, r);
    r = syscall2(SYS_pivot_root, D, D "/d");
    check("both_dirs_eperm", r == -EPERM, r);

    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/d", AT_REMOVEDIR);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
