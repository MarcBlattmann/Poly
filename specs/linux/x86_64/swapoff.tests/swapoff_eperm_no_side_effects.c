/* A refused swapoff (EPERM, capabilities dropped) has no side effects:
 * the file keeps its size (8192), mode (0644) and bytes, a file holding
 * a swap signature is refused the same way, a missing name is not
 * created, and no file descriptor is consumed (the next open gets 3, the
 * lowest free number). */
#include "poly_ipc.h"

#define SYS_swapoff 168
#define SYS_newfstatat 262

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

/* Empty all three capability sets: any user may do that, and root then
 * fails the privilege check like everybody else. */
static i64 drop_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

#define F "/tmp/swapoff_eperm_side_file"
#define M "/tmp/swapoff_eperm_side_missing"

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
    for (int i = 0; i < 8192; i++) data[i] = (unsigned char)(i * 7 + 1);
    /* A swap signature (mkswap(8) layout): version 1, last_page 1. */
    memcpy(data + 4086, "SWAPSPACE2", 10);
    *(unsigned *)(data + 1024) = 1;
    *(unsigned *)(data + 1028) = 1;
    *(unsigned *)(data + 1032) = 0;
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0644);
    check("create", fd == 3, fd);
    check("write", syscall3(SYS_write, fd, data, sizeof data) == (i64)sizeof data, 0);
    syscall1(SYS_close, fd);
    syscall3(SYS_unlinkat, AT_FDCWD, M, 0);
    check("drop_caps", drop_caps() == 0, 0);

    i64 r = syscall1(SYS_swapoff, F);
    check("signature_file_eperm", r == -EPERM, r);
    r = syscall1(SYS_swapoff, M);
    check("missing_eperm", r == -EPERM, r);

    r = syscall4(SYS_newfstatat, AT_FDCWD, F, st, 0);
    check("stat", r == 0, r);
    check("size_kept", size_of(st) == 8192, size_of(st));
    check("mode_kept", (mode_of(st) & 07777) == 0644, mode_of(st));
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
