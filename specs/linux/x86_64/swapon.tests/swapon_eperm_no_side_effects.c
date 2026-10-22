/* A swapon refused with EPERM (1) (every capability dropped with capset
 * first) leaves the target alone: its size, mode and contents are
 * unchanged, no file descriptor is used up (the next open gets the lowest
 * free number, 3), and a missing target is not created. Dropping only
 * the effective set (permitted kept) is enough for EPERM. */
#include "poly_ipc.h"

#define SYS_swapon 167
#define SYS_newfstatat 262
#define SYS_pread64 17

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};
struct k_stat {
    u64 dev, ino, nlink;
    unsigned mode, uid, gid, pad0;
    u64 rdev;
    i64 size, blksize, blocks;
    u64 times[6];
    i64 unused[3];
};

#define F "/tmp/swapon_eperm_noside_file"
#define M "/tmp/swapon_eperm_noside_missing"

static char page[8192];
static char back[8192];

int test_main(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    struct k_stat st;
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, M, 0);
    for (int i = 0; i < 8192; i++) page[i] = (char)(i * 7 + 1);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0640);
    check("create", fd == 3, fd);
    i64 n = syscall3(SYS_write, fd, page, sizeof page);
    check("write", n == 8192, n);
    close((int)fd);

    i64 r = syscall2(SYS_capget, &h, d);
    check("capget", r == 0, r);
    d[0].effective = 0;
    d[1].effective = 0;
    r = syscall2(SYS_capset, &h, d);
    check("drop_effective", r == 0, r);

    r = syscall2(SYS_swapon, F, 0);
    check("file_eperm", r == -EPERM, r);
    r = syscall2(SYS_swapon, M, 0x8000 | 3);
    check("missing_eperm", r == -EPERM, r);

    r = syscall4(SYS_newfstatat, AT_FDCWD, F, &st, 0);
    check("stat", r == 0, r);
    check("size_unchanged", st.size == 8192, st.size);
    check("mode_unchanged", (st.mode & 07777) == 0640, st.mode);
    fd = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY, 0);
    check("next_fd_is_3", fd == 3, fd);
    n = syscall4(SYS_pread64, fd, back, sizeof back, 0);
    check("read_back", n == 8192, n);
    int same = 1;
    for (int i = 0; i < 8192; i++) same &= page[i] == back[i];
    check("contents_unchanged", same, 0);
    close((int)fd);
    r = syscall4(SYS_newfstatat, AT_FDCWD, M, &st, 0);
    check("missing_not_created", r == -ENOENT, r);

    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
