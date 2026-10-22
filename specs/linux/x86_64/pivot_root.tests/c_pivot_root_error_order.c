/* When both arguments are bad, new_root's error is the one reported:
 * missing new_root with a file or bad put_old is ENOENT, a file
 * new_root with a missing put_old ENOTDIR, a bad new_root pointer with a
 * missing put_old EFAULT; a bad put_old pointer with a good new_root is
 * EFAULT. All with capabilities dropped, so none of them is EPERM. */
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
#define D "/tmp/c_pivot_root_order"

int test_main(void) {
    syscall1(SYS_chdir, "/");
    check("drop_caps", drop_caps() == 0, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    make_file(D "/f");

    i64 r = syscall2(SYS_pivot_root, D "/missing", D "/f");
    check("missing_then_file_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, D "/missing", 8);
    check("missing_then_bad_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, D "/f", D "/missing");
    check("file_then_missing_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, D "/f", 8);
    check("file_then_bad_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, 8, D "/missing");
    check("bad_then_missing_efault", r == -EFAULT, r);
    r = syscall2(SYS_pivot_root, 0, 0);
    check("null_null_efault", r == -EFAULT, r);
    r = syscall2(SYS_pivot_root, D, 8);
    check("dir_then_bad_efault", r == -EFAULT, r);

    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
