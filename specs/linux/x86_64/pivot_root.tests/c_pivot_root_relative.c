/* Relative paths are resolved from the current directory: in a
 * directory holding d (a directory) and f (a file), with capabilities
 * dropped, ("d", "d") and (".", "d") and ("..", ".") are EPERM, ("f",
 * "d") ENOTDIR, ("d", "f") ENOTDIR, ("nope", "d") ENOENT; the current
 * directory stays where it was. */
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
#define D "/tmp/c_pivot_root_rel"

int test_main(void) {
    syscall1(SYS_chdir, "/");
    check("drop_caps", drop_caps() == 0, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/d", AT_REMOVEDIR);
    syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, D "/d", 0755);
    make_file(D "/f");
    check("chdir", syscall1(SYS_chdir, D) == 0, 0);

    i64 r = syscall2(SYS_pivot_root, "d", "d");
    check("d_d_eperm", r == -EPERM, r);
    r = syscall2(SYS_pivot_root, ".", "d");
    check("dot_d_eperm", r == -EPERM, r);
    r = syscall2(SYS_pivot_root, "..", ".");
    check("dotdot_dot_eperm", r == -EPERM, r);
    r = syscall2(SYS_pivot_root, "f", "d");
    check("f_d_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, "d", "f");
    check("d_f_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, "nope", "d");
    check("nope_d_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, "d", "d/nope");
    check("d_nope_enoent", r == -ENOENT, r);

    char cwd[64];
    r = syscall2(SYS_getcwd, cwd, sizeof cwd);
    check("cwd_unchanged", r == (i64)sizeof D && str_eq(cwd, D), r);
    syscall1(SYS_chdir, "/");
    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/d", AT_REMOVEDIR);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
