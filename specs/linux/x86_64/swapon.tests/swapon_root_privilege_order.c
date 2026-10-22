/* For root the order of checks is: flags, then CAP_SYS_ADMIN, then the
 * path. With all capabilities a directory is EISDIR (21) for valid flags
 * (bits 32-63 ignored) and EINVAL (22) for an unknown flag bit. In a
 * child that dropped only CAP_SYS_ADMIN (21) from its effective set,
 * keeping every other capability, the same directory, a missing file
 * and a bad pointer are EPERM (1), while an unknown flag bit is still
 * EINVAL. The parent keeps its capabilities.
 * poly-diff: needs=root (only root has CAP_SYS_ADMIN to lose)
 */
#include "poly_ipc.h"

#define EISDIR 21
#define CAP_SYS_ADMIN 21

#define SYS_swapon 167
#define SYS_swapoff 168

/* A failed swapon must not have enabled anything; if it did, undo it. */
static __attribute__((unused)) void undo(const char *path, i64 r) {
    if (r == 0) syscall1(SYS_swapoff, path);
}

static __attribute__((unused)) void make_file(const char *path, const char *data, i64 len) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, 0600);
    if (len > 0) syscall3(SYS_write, fd, data, len);
    syscall1(SYS_close, fd);
}

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

int test_main(void) {
    i64 r = syscall2(SYS_swapon, "/tmp", 0);
    check("dir_eisdir", r == -EISDIR, r);
    r = syscall2(SYS_swapon, "/tmp", 0x7ffff);
    check("dir_all_valid_eisdir", r == -EISDIR, r);
    r = syscall2(SYS_swapon, "/tmp", (i64)0xffffffff00000000UL);
    check("dir_upper_bits_eisdir", r == -EISDIR, r);
    r = syscall2(SYS_swapon, "/tmp", 0x80000);
    check("dir_bad_flag_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, 8, 0x100000);
    check("bad_pointer_bad_flag_einval", r == -EINVAL, r);

    i64 pid = fork();
    if (pid == 0) {
        struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
        struct cap_data d[2];
        i64 c = syscall2(SYS_capget, &h, d);
        check("child_capget", c == 0, c);
        d[0].effective &= ~(1u << CAP_SYS_ADMIN);
        c = syscall2(SYS_capset, &h, d);
        check("child_drop_sys_admin", c == 0, c);
        if (c != 0) exit_group(1);
        c = syscall2(SYS_swapon, "/tmp", 0);
        check("child_dir_eperm", c == -EPERM, c);
        c = syscall2(SYS_swapon, "/tmp/swapon_root_order_missing", 0);
        check("child_missing_eperm", c == -EPERM, c);
        c = syscall2(SYS_swapon, 8, 0);
        check("child_bad_pointer_eperm", c == -EPERM, c);
        c = syscall2(SYS_swapon, "/tmp", 0x80000);
        check("child_bad_flag_einval", c == -EINVAL, c);
        exit_group(failed ? 1 : 0);
    }
    int code = pid > 0 ? wait_exit(pid) : -1;
    check("fork", pid > 0, pid);
    check("child", code == 0, code);

    r = syscall2(SYS_swapon, "/tmp", 0);
    check("parent_still_eisdir", r == -EISDIR, r);
    return finish();
}
