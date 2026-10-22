/* For root the privilege check comes before the path. With all
 * capabilities a directory is EISDIR (21) and a missing file ENOENT (2).
 * In a child that dropped only CAP_SYS_ADMIN (21) from its effective
 * set, keeping every other capability, the same directory, a missing
 * file, a regular file, NULL and a bad pointer are EPERM (1). The parent
 * keeps its capabilities and still gets EISDIR.
 * poly-diff: needs=root (only root has CAP_SYS_ADMIN to lose)
 */
#include "poly_ipc.h"

#define EISDIR 21
#define CAP_SYS_ADMIN 21

#define SYS_swapoff 168

#define F "/tmp/swapoff_root_order_file"
#define M "/tmp/swapoff_root_order_missing"

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0600));
    i64 r = syscall1(SYS_swapoff, "/tmp");
    check("dir_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_swapoff, M);
    check("missing_enoent", r == -ENOENT, r);
    r = syscall1(SYS_swapoff, F);
    check("file_einval", r == -EINVAL, r);

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
        c = syscall1(SYS_swapoff, "/tmp");
        check("child_dir_eperm", c == -EPERM, c);
        c = syscall1(SYS_swapoff, M);
        check("child_missing_eperm", c == -EPERM, c);
        c = syscall1(SYS_swapoff, F);
        check("child_file_eperm", c == -EPERM, c);
        c = syscall1(SYS_swapoff, 0);
        check("child_null_eperm", c == -EPERM, c);
        c = syscall1(SYS_swapoff, 8);
        check("child_bad_pointer_eperm", c == -EPERM, c);
        exit_group(failed ? 1 : 0);
    }
    int code = pid > 0 ? wait_exit(pid) : -1;
    check("fork", pid > 0, pid);
    check("child", code == 0, code);

    r = syscall1(SYS_swapoff, "/tmp");
    check("parent_still_eisdir", r == -EISDIR, r);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
