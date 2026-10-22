/* acct without CAP_SYS_PACCT (all capabilities dropped with capset first,
 * so root and a normal user behave the same): naming an existing, writable
 * regular file is EPERM (1). Nothing is switched on: a child that exits
 * afterwards adds nothing to the file, which keeps its size 0, its mode
 * and its owner. */
#include "poly_ipc.h"

#define SYS_acct 163
#define SYS_newfstatat 262

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

/* Empty all three capability sets: any user may do that, and root then
 * fails the privilege check like everybody else. */
static i64 drop_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

static __attribute__((unused)) i64 file_size(const char *path) {
    struct k_stat st;
    i64 r = syscall4(SYS_newfstatat, AT_FDCWD, path, &st, 0);
    return r < 0 ? r : st.size;
}

static __attribute__((unused)) void make_file(const char *path, unsigned mode) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, mode));
}
#define F "/tmp/acct_eperm_file"

int test_main(void) {
    make_file(F, 0644);
    struct k_stat before, after;
    syscall4(SYS_newfstatat, AT_FDCWD, F, &before, 0);
    check("drop_caps", drop_caps() == 0, 0);
    i64 r = syscall1(SYS_acct, F);
    check("file_eperm", r == -EPERM, r);
    i64 pid = fork();
    if (pid == 0) exit_group(3);
    int code = wait_exit(pid);
    check("child_exit_3", code == 3, code);
    r = syscall4(SYS_newfstatat, AT_FDCWD, F, &after, 0);
    check("file_still_empty", r == 0 && after.size == 0, after.size);
    check("mode_unchanged", after.mode == before.mode, after.mode);
    check("owner_unchanged", after.uid == before.uid && after.gid == before.gid, after.uid);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
