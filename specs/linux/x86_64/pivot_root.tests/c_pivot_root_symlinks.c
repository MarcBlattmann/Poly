/* Both paths follow symlinks, also in the last component: a symlink to
 * a directory is the directory (EPERM without capabilities), a symlink
 * to a file ENOTDIR, a dangling symlink ENOENT, a symlink to itself
 * ELOOP (40), for new_root and for put_old alike. */
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
#define D "/tmp/c_pivot_root_sym"

static void clean(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, D "/ld", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/lf", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/ln", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/loop", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/d", AT_REMOVEDIR);
}

int test_main(void) {
    syscall1(SYS_chdir, "/");
    check("drop_caps", drop_caps() == 0, 0);
    clean();
    syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, D "/d", 0755);
    make_file(D "/f");
    syscall3(SYS_symlinkat, "d", AT_FDCWD, D "/ld");
    syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/lf");
    syscall3(SYS_symlinkat, "nothere", AT_FDCWD, D "/ln");
    syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop");

    i64 r = syscall2(SYS_pivot_root, D "/ld", D "/ld");
    check("dir_links_eperm", r == -EPERM, r);
    r = syscall2(SYS_pivot_root, D "/ld/", D "/ld/");
    check("dir_links_slash_eperm", r == -EPERM, r);
    r = syscall2(SYS_pivot_root, D "/lf", D "/d");
    check("new_file_link_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, D "/ln", D "/d");
    check("new_dangling_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, D "/loop", D "/d");
    check("new_loop_eloop", r == -ELOOP, r);
    r = syscall2(SYS_pivot_root, D "/d", D "/lf");
    check("old_file_link_enotdir", r == -ENOTDIR, r);
    r = syscall2(SYS_pivot_root, D "/d", D "/ln");
    check("old_dangling_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, D "/d", D "/loop");
    check("old_loop_eloop", r == -ELOOP, r);

    clean();
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
