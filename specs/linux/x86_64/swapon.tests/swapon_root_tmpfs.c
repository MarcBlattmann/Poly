/* A file on tmpfs cannot be swapped to: even with a well-formed swap
 * header (signature "SWAPSPACE2" at bytes 4086-4095, version 1 at byte
 * 1024, last_page 15 at 1028, no bad pages, 16 pages long) swapon is
 * EINVAL (22), with flags 0 and with SWAP_FLAG_PREFER. The file is left
 * as it was. Runs on a tmpfs in a mount namespace of its own.
 * poly-diff: needs=root (only root may mount and get past CAP_SYS_ADMIN)
 */
#include "poly_ipc.h"

#define SYS_pread64 17

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

#define D "/tmp/swapon_root_tmpfs"
#define F D "/swapfile"
#define PAGES 16

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

static char img[PAGES * 4096];
static char back[PAGES * 4096];

static void put32(int off, unsigned v) {
    img[off] = (char)v;
    img[off + 1] = (char)(v >> 8);
    img[off + 2] = (char)(v >> 16);
    img[off + 3] = (char)(v >> 24);
}

static void body(void) {
    memset(img, 0, sizeof img);
    const char *sig = "SWAPSPACE2";
    for (int i = 0; i < 10; i++) img[4086 + i] = sig[i];
    put32(1024, 1);
    put32(1028, PAGES - 1);
    make_file(F, img, sizeof img);

    i64 r = syscall2(SYS_swapon, F, 0);
    check("tmpfs_einval", r == -EINVAL, r);
    undo(F, r);
    r = syscall2(SYS_swapon, F, 0x8000 | 1);
    check("tmpfs_prefer_einval", r == -EINVAL, r);
    undo(F, r);

    i64 fd = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY, 0);
    i64 n = syscall4(SYS_pread64, fd, back, sizeof back, 0);
    syscall1(SYS_close, fd);
    check("size_unchanged", n == (i64)sizeof img, n);
    int same = 1;
    for (int i = 0; i < (int)sizeof img; i++) same &= img[i] == back[i];
    check("contents_unchanged", same, 0);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    int code = in_private_tmpfs(body);
    check("child", code == 0, code);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
