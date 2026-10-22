/* A privileged swapoff that fails (EINVAL for a file that is not a swap
 * area, ENOENT for a missing name) has no side effects: the file keeps
 * its size (8192), mode (0640) and bytes, the missing name is not
 * created, and no file descriptor stays open (the next open gets 3, the
 * lowest free number), also after twenty failed calls in a row.
 * poly-diff: needs=root (only root gets past the CAP_SYS_ADMIN check)
 */
#include "poly_ipc.h"

#define SYS_swapoff 168
#define SYS_newfstatat 262

#define F "/tmp/swapoff_root_side_file"
#define M "/tmp/swapoff_root_side_missing"

static unsigned char data[8192];
static unsigned char back[8192];
static unsigned char st[256];

static unsigned mode_of(const unsigned char *s) { return *(const unsigned *)(s + 24); }
static i64 size_of(const unsigned char *s) { return *(const i64 *)(s + 48); }

static int same(const unsigned char *a, const unsigned char *b, int n) {
    for (int i = 0; i < n; i++)
        if (a[i] != b[i]) return 0;
    return 1;
}

int test_main(void) {
    for (int i = 0; i < 8192; i++) data[i] = (unsigned char)(i * 13 + 5);
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0640);
    check("create", fd == 3, fd);
    check("write", syscall3(SYS_write, fd, data, sizeof data) == (i64)sizeof data, 0);
    syscall1(SYS_close, fd);
    syscall3(SYS_unlinkat, AT_FDCWD, M, 0);

    i64 r = syscall1(SYS_swapoff, F);
    check("file_einval", r == -EINVAL, r);
    r = syscall1(SYS_swapoff, M);
    check("missing_enoent", r == -ENOENT, r);
    int all = 1;
    for (int i = 0; i < 20; i++)
        if (syscall1(SYS_swapoff, F) != -EINVAL) all = 0;
    check("repeated_einval", all, 0);

    r = syscall4(SYS_newfstatat, AT_FDCWD, F, st, 0);
    check("stat", r == 0, r);
    check("size_kept", size_of(st) == 8192, size_of(st));
    check("mode_kept", (mode_of(st) & 07777) == 0640, mode_of(st));
    fd = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY, 0);
    check("lowest_fd", fd == 3, fd);
    r = syscall3(SYS_read, fd, back, sizeof back);
    check("read_back", r == (i64)sizeof back, r);
    check("bytes_kept", same(back, data, sizeof data), 0);
    syscall1(SYS_close, fd);
    r = syscall4(SYS_newfstatat, AT_FDCWD, M, st, 0);
    check("missing_not_created", r == -ENOENT, r);

    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
