/* A swapon that failed leaves nothing for swapoff to stop: on a tmpfs
 * (which cannot hold swap) a file with a well-formed swap signature is
 * EINVAL (22) for swapon, then EINVAL for swapoff, which does not modify
 * it; swapon with an unknown flag bit (EINVAL) followed by swapoff is
 * EINVAL too. Runs on a tmpfs in a mount namespace of its own.
 * poly-diff: needs=root (only root may mount and get past CAP_SYS_ADMIN)
 */
#include "poly_ipc.h"

#define SYS_swapon 167
#define SYS_swapoff 168

#define D "/tmp/swapoff_root_afterswapon"
#define F D "/swapfile"

static void make_file(const char *path, const void *data, i64 len, int mode) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, mode);
    if (len > 0) syscall3(SYS_write, fd, data, len);
    syscall1(SYS_close, fd);
}

#define SYS_unshare 272
#define CLONE_NEWNS 0x00020000

/* Run body in a child with a mount namespace of its own (every mount
 * private) and a fresh tmpfs on D. The child never calls body unless
 * all of that worked. Returns the child's exit code. */
static int in_private_tmpfs(void (*body)(void)) {
    i64 pid = fork();
    if (pid == 0) {
        i64 r = syscall1(SYS_unshare, CLONE_NEWNS);
        check("unshare_newns", r == 0, r);
        if (r != 0) exit_group(1);
        r = syscall5(SYS_mount, "none", "/", 0, MS_REC | MS_PRIVATE, 0);
        check("make_private", r == 0, r);
        if (r != 0) exit_group(1);
        r = syscall5(SYS_mount, "x", D, "tmpfs", 0, "size=1m");
        check("mount_tmpfs", r == 0, r);
        if (r != 0) exit_group(1);
        body();
        syscall2(SYS_umount2, D, MNT_DETACH);
        exit_group(failed ? 1 : 0);
    }
    int code = pid > 0 ? wait_exit(pid) : -1;
    check("fork", pid > 0, pid);
    return code;
}

static unsigned char buf[16 * 4096];
static unsigned char back[16 * 4096];

static int same(const unsigned char *a, const unsigned char *b, int n) {
    for (int i = 0; i < n; i++)
        if (a[i] != b[i]) return 0;
    return 1;
}

static void body(void) {
    /* A swap signature (mkswap(8) layout): version 1, last_page 15. */
    memcpy(buf + 4086, "SWAPSPACE2", 10);
    *(unsigned *)(buf + 1024) = 1;
    *(unsigned *)(buf + 1028) = 15;
    make_file(F, buf, sizeof buf, 0600);

    i64 r = syscall2(SYS_swapon, F, 0);
    check("swapon_tmpfs_einval", r == -EINVAL, r);
    if (r == 0) syscall1(SYS_swapoff, F);
    r = syscall1(SYS_swapoff, F);
    check("swapoff_einval", r == -EINVAL, r);
    r = syscall2(SYS_swapon, F, 0x80000);
    check("swapon_bad_flag_einval", r == -EINVAL, r);
    if (r == 0) syscall1(SYS_swapoff, F);
    r = syscall1(SYS_swapoff, F);
    check("swapoff_again_einval", r == -EINVAL, r);

    i64 fd = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY, 0);
    check("open", fd >= 0, fd);
    r = syscall3(SYS_read, fd, back, sizeof back);
    check("read_back", r == (i64)sizeof back, r);
    check("bytes_kept", same(back, buf, sizeof buf), 0);
    syscall1(SYS_close, fd);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    int code = in_private_tmpfs(body);
    check("child", code == 0, code);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
