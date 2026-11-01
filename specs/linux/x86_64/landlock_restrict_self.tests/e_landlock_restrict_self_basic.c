/* landlock_restrict_self(2) enforces a ruleset on the calling thread
 * (and its future children); it cannot be undone. (A caller without
 * CAP_SYS_ADMIN must set no_new_privs first; this test sets it for any
 * user.) Restrictions stack: enforcing a second ruleset only narrows further.
 * The ruleset fd may be closed after enforcement; restrictions are
 * inherited across fork. Errors: unknown flags EINVAL; a closed fd EBADF;
 * a descriptor that is not a ruleset EBADFD.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_pread64 17
#define SYS_mremap 25
#define SYS_madvise 28
#define SYS_dup 32
#define SYS_ftruncate 77
#define SYS_fchdir 81
#define SYS_mlock 149
#define SYS_name_to_handle_at 303
#define SYS_open_by_handle_at 304
#define SYS_fanotify_init 300
#define SYS_fanotify_mark 301
#define SYS_userfaultfd 323
#define SYS_memfd_create 319
#define SYS_landlock_create_ruleset 444
#define SYS_landlock_add_rule 445
#define SYS_landlock_restrict_self 446
#define SYS_memfd_secret 447
#define SYS_cachestat 451
#define SYS_mseal 462
#define SYS_statx 332
#define SYS_fallocate 285
#define SYS_sendfile 40
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EBADF 9
#define ENOMEM 12
#define EFAULT 14
#define EINVAL 22
#define E2BIG 7
#define ENOMSG 42
#define EBADFD 77
#define EOVERFLOW 75
#define ESTALE 116
/* asm-generic/fcntl.h, linux/fcntl.h */
#define O_WRONLY 1
#define O_APPEND 02000
#define O_PATH 010000000
#define AT_SYMLINK_NOFOLLOW 0x100
#define AT_SYMLINK_FOLLOW 0x400
#define AT_EMPTY_PATH 0x1000
#define AT_HANDLE_FID 0x200
#define AT_HANDLE_MNT_ID_UNIQUE 0x001
#define MAX_HANDLE_SZ 128
/* asm-generic/mman-common.h */
#define MREMAP_MAYMOVE 1
#define MADV_WILLNEED 3
#define MADV_DONTNEED 4
#define MAP_FIXED 0x10
/* linux/prctl.h */
#define PR_SET_NO_NEW_PRIVS 38

/* struct file_handle (linux/fcntl.h via fs): header then f_handle[]. */
struct k_file_handle {
    unsigned handle_bytes;
    int handle_type;
    unsigned char f_handle[MAX_HANDLE_SZ];
};

static __attribute__((unused)) i64 open3(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static __attribute__((unused)) i64 unlink_path(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, 0); }
static __attribute__((unused)) void make_file(const char *path, const char *text, i64 mode) {
    unlink_path(path);
    i64 fd = open3(path, O_CREAT | O_RDWR | O_EXCL, mode);
    if (fd >= 0) {
        if (text) write((int)fd, text, str_len(text));
        close((int)fd);
    }
}
static __attribute__((unused)) i64 fork_sync(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    return syscall5(SYS_clone, 0x4000 /* CLONE_VFORK */ | SIGCHLD, 0, 0, 0, 0);
}
static __attribute__((unused)) int drop_to_user(void) {
    if (syscall3(SYS_setresgid, 1000, 1000, 1000) != 0) return 0;
    if (syscall3(SYS_setresuid, 1000, 1000, 1000) != 0) return 0;
    return 1;
}
static __attribute__((unused)) void *map_anon(u64 len, int prot, int flags) {
    return (void *)syscall6(SYS_mmap, 0, (i64)len, prot, flags | MAP_ANONYMOUS, -1, 0);
}
#define BAD_PTR(p) ((u64)(p) > (u64)-4096L)
/* linux/landlock.h */
#define LANDLOCK_CREATE_RULESET_VERSION (1U << 0)
#define LANDLOCK_CREATE_RULESET_ERRATA (1U << 1)
#define LANDLOCK_RULE_PATH_BENEATH 1
#define LANDLOCK_RULE_NET_PORT 2
#define LANDLOCK_ACCESS_FS_EXECUTE (1UL << 0)
#define LANDLOCK_ACCESS_FS_WRITE_FILE (1UL << 1)
#define LANDLOCK_ACCESS_FS_READ_FILE (1UL << 2)
#define LANDLOCK_ACCESS_FS_READ_DIR (1UL << 3)
#define LANDLOCK_ACCESS_FS_REMOVE_DIR (1UL << 4)
#define LANDLOCK_ACCESS_FS_REMOVE_FILE (1UL << 5)
#define LANDLOCK_ACCESS_FS_MAKE_CHAR (1UL << 6)
#define LANDLOCK_ACCESS_FS_MAKE_DIR (1UL << 7)
#define LANDLOCK_ACCESS_FS_MAKE_REG (1UL << 8)
#define LANDLOCK_ACCESS_FS_MAKE_SOCK (1UL << 9)
#define LANDLOCK_ACCESS_FS_MAKE_FIFO (1UL << 10)
#define LANDLOCK_ACCESS_FS_MAKE_BLOCK (1UL << 11)
#define LANDLOCK_ACCESS_FS_MAKE_SYM (1UL << 12)
#define LANDLOCK_ACCESS_FS_REFER (1UL << 13)
#define LANDLOCK_ACCESS_FS_TRUNCATE (1UL << 14)
#define LANDLOCK_ACCESS_FS_IOCTL_DEV (1UL << 15)
#define LANDLOCK_ACCESS_NET_BIND_TCP (1UL << 0)
#define LANDLOCK_ACCESS_NET_CONNECT_TCP (1UL << 1)
#define LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET (1UL << 0)
#define LANDLOCK_SCOPE_SIGNAL (1UL << 1)
#define LANDLOCK_RESTRICT_SELF_LOG_SAME_EXEC_OFF (1U << 0)
#define LANDLOCK_RESTRICT_SELF_LOG_NEW_EXEC_ON (1U << 1)
#define LANDLOCK_RESTRICT_SELF_LOG_SUBDOMAINS_OFF (1U << 2)
/* struct landlock_ruleset_attr (24 bytes in ABI 6+). */
struct k_ll_ruleset_attr {
    u64 handled_access_fs, handled_access_net, scoped;
};
/* struct landlock_path_beneath_attr (packed, 12 bytes). */
struct __attribute__((packed)) k_ll_path_beneath {
    u64 allowed_access;
    int parent_fd;
};
/* struct landlock_net_port_attr (16 bytes). */
struct k_ll_net_port {
    u64 allowed_access;
    u64 port;
};
static __attribute__((unused)) i64 ll_create(struct k_ll_ruleset_attr *a, u64 size, unsigned flags) {
    return syscall3(SYS_landlock_create_ruleset, a, size, flags);
}
static __attribute__((unused)) i64 ll_add(int fd, int type, const void *attr, unsigned flags) {
    return syscall4(SYS_landlock_add_rule, fd, type, attr, flags);
}
static __attribute__((unused)) i64 ll_restrict(int fd, unsigned flags) {
    return syscall2(SYS_landlock_restrict_self, fd, flags);
}
static __attribute__((unused)) i64 no_new_privs(void) { return syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0); }

#define A "/tmp/e_landlock_restrict_self_basic"

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, A, 0755);
    make_file(A "/f", "1", 0644);
    make_file(A "/g", "2", 0644);
    struct k_ll_ruleset_attr a = { LANDLOCK_ACCESS_FS_READ_FILE, 0, 0 };
    i64 rs = ll_create(&a, sizeof a, 0);
    /* no_new_privs first, so the checks below are reached by any user. */
    check("no_new_privs", no_new_privs() == 0, 0);
    i64 r = ll_restrict((int)rs, 8);
    check("bad_flags_einval", r == -EINVAL, r);
    r = ll_restrict(99, 0);
    check("closed_ebadf", r == -EBADF, r);
    r = ll_restrict(1, 0);
    check("not_ruleset_ebadfd", r == -EBADFD, r);

    i64 pid = fork_sync();
    if (pid == 0) {
        int bad = 0;
        /* Allow A; then stack a second ruleset allowing only A/f. */
        i64 d = open3(A, O_PATH, 0);
        struct k_ll_path_beneath pb = { LANDLOCK_ACCESS_FS_READ_FILE, (int)d };
        ll_add((int)rs, LANDLOCK_RULE_PATH_BENEATH, &pb, 0);
        if (ll_restrict((int)rs, 0) != 0) bad |= 4;
        close((int)rs);
        if (open3(A "/g", O_RDONLY, 0) < 0) bad |= 8;
        if (open3("/etc/hostname", O_RDONLY, 0) != -EACCES && open3("/proc/self/status", O_RDONLY, 0) != -EACCES) bad |= 16;
        i64 rs2 = ll_create(&a, sizeof a, 0);
        i64 f = open3(A "/f", O_PATH, 0);
        pb.parent_fd = (int)f;
        ll_add((int)rs2, LANDLOCK_RULE_PATH_BENEATH, &pb, 0);
        if (ll_restrict((int)rs2, 0) != 0) bad |= 32;
        if (open3(A "/f", O_RDONLY, 0) < 0) bad |= 64;
        if (open3(A "/g", O_RDONLY, 0) != -EACCES) bad |= 128;
        i64 gc = fork_sync();
        if (gc == 0) exit_group(open3(A "/g", O_RDONLY, 0) == -EACCES ? 0 : 1);
        if (wait_exit(gc) != 0) bad |= 256;
        exit_group(bad);
    }
    int st = wait_exit(pid);
    check("enforce_stack_inherit", st == 0, st);
    i64 g = open3(A "/g", O_RDONLY, 0);
    check("parent_unaffected", g >= 0, g);
    close((int)g);
    close((int)rs);
    unlink_path(A "/f");
    unlink_path(A "/g");
    syscall3(SYS_unlinkat, AT_FDCWD, A, AT_REMOVEDIR);
    return finish();
}
