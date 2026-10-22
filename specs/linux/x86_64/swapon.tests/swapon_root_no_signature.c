/* A regular file without a swap signature (the 10 bytes "SWAPSPACE2" at
 * the end of its first 4096-byte page) is EINVAL (22): two pages of
 * zeros, two pages of text, a file of 100 bytes, and also with
 * SWAP_FLAG_PREFER or SWAP_FLAG_DISCARD. The file is left as it was
 * (same size, same bytes).
 * poly-diff: needs=root (only root gets past the CAP_SYS_ADMIN check)
 */
#include "poly_ipc.h"

#define SYS_pread64 17

#define SYS_swapon 167
#define SYS_newfstatat 262
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

#define Z "/tmp/swapon_root_nosig_zero"
#define T "/tmp/swapon_root_nosig_text"
#define SM "/tmp/swapon_root_nosig_small"

static char buf[8192];
static char back[8192];
static char st[256];

static int same_as(const char *path, const char *want, i64 len) {
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_RDONLY, 0);
    if (fd < 0) return 0;
    i64 n = syscall4(SYS_pread64, fd, back, sizeof back, 0);
    syscall1(SYS_close, fd);
    if (n != len) return 0;
    for (i64 i = 0; i < len; i++)
        if (back[i] != want[i]) return 0;
    return 1;
}

int test_main(void) {
    memset(buf, 0, sizeof buf);
    make_file(Z, buf, sizeof buf);
    i64 r = syscall2(SYS_swapon, Z, 0);
    check("zeros_einval", r == -EINVAL, r);
    undo(Z, r);
    r = syscall2(SYS_swapon, Z, 0x8000 | 7);
    check("zeros_prefer_einval", r == -EINVAL, r);
    undo(Z, r);
    r = syscall2(SYS_swapon, Z, 0x10000);
    check("zeros_discard_einval", r == -EINVAL, r);
    undo(Z, r);
    check("zeros_unchanged", same_as(Z, buf, sizeof buf), 0);

    for (int i = 0; i < 8192; i++) buf[i] = (char)('a' + i % 26);
    make_file(T, buf, sizeof buf);
    r = syscall2(SYS_swapon, T, 0);
    check("text_einval", r == -EINVAL, r);
    undo(T, r);
    check("text_unchanged", same_as(T, buf, sizeof buf), 0);

    make_file(SM, buf, 100);
    r = syscall2(SYS_swapon, SM, 0);
    check("small_einval", r == -EINVAL, r);
    undo(SM, r);
    check("small_unchanged", same_as(SM, buf, 100), 0);
    r = syscall4(SYS_newfstatat, AT_FDCWD, SM, st, 0);
    check("small_stat", r == 0, r);

    syscall3(SYS_unlinkat, AT_FDCWD, Z, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, T, 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, SM, 0) == 0, 0);
    return finish();
}
