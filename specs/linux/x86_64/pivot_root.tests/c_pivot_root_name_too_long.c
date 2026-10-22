/* Path length limits apply to both arguments before the privilege
 * check: a path of 4096 bytes before its NUL is ENAMETOOLONG (36), and
 * so is a path with a 256-byte component; a 255-byte component is fine
 * (the name then just does not exist: ENOENT). Capabilities dropped. */
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
#define D "/tmp/c_pivot_root_ntl"

static char path[4200];

static void fill(char *p, int n) {
    for (int i = 0; i < n; i++) p[i] = 'a';
    p[n] = 0;
}

int test_main(void) {
    syscall1(SYS_chdir, "/");
    check("drop_caps", drop_caps() == 0, 0);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);

    /* "/" followed by "./" pairs and filler up to 4096 bytes */
    path[0] = '/';
    int n = 1;
    while (n + 2 <= 4096) {
        path[n++] = '.';
        path[n++] = '/';
    }
    while (n < 4096) path[n++] = 'x';
    path[n] = 0;
    i64 r = syscall2(SYS_pivot_root, path, D);
    check("new_4096_enametoolong", r == -ENAMETOOLONG, r);
    r = syscall2(SYS_pivot_root, D, path);
    check("old_4096_enametoolong", r == -ENAMETOOLONG, r);

    memcpy(path, D "/", sizeof D);
    fill(path + sizeof D, 256);
    r = syscall2(SYS_pivot_root, path, D);
    check("new_component_256_enametoolong", r == -ENAMETOOLONG, r);
    r = syscall2(SYS_pivot_root, D, path);
    check("old_component_256_enametoolong", r == -ENAMETOOLONG, r);
    fill(path + sizeof D, 255);
    r = syscall2(SYS_pivot_root, path, D);
    check("new_component_255_enoent", r == -ENOENT, r);
    r = syscall2(SYS_pivot_root, D, path);
    check("old_component_255_enoent", r == -ENOENT, r);

    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
