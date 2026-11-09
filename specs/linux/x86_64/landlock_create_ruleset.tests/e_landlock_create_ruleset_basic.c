/* landlock_create_ruleset(2) with LANDLOCK_CREATE_RULESET_VERSION (attr
 * NULL, size 0) returns the Landlock ABI version (7 on Linux 6.18; the
 * test requires at least 6, which has scopes); with ..._ERRATA it returns
 * a bitmask of fixed errata (>= 0). Otherwise it makes a ruleset from
 * struct landlock_ruleset_attr {handled_access_fs, handled_access_net,
 * scoped} and returns a descriptor (always close-on-exec, O_RDWR) that
 * cannot be read. Shorter sizes from older ABIs (8: fs only, 16: fs and
 * net) are accepted; a larger size needs a zero tail.
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

int test_main(void) {
    i64 v = ll_create(0, 0, LANDLOCK_CREATE_RULESET_VERSION);
    check("abi_version", v >= 6, v);
    i64 e = ll_create(0, 0, LANDLOCK_CREATE_RULESET_ERRATA);
    check("errata", e >= 0, e);
    struct {
        struct k_ll_ruleset_attr a;
        u64 ext;
    } b;
    memset(&b, 0, sizeof b);
    b.a.handled_access_fs = LANDLOCK_ACCESS_FS_READ_FILE;
    i64 fd = ll_create(&b.a, sizeof b.a, 0);
    check("ruleset_fd", fd == 3, fd);
    check("cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == FD_CLOEXEC, syscall2(SYS_fcntl, fd, F_GETFD));
    check("rdwr", syscall2(SYS_fcntl, fd, F_GETFL) == O_RDWR, syscall2(SYS_fcntl, fd, F_GETFL));
    char buf[8];
    check("read_einval", read((int)fd, buf, 8) == -EINVAL, 0);
    close((int)fd);
    fd = ll_create(&b.a, 8, 0);
    check("size_8_ok", fd == 3, fd);
    close((int)fd);
    fd = ll_create(&b.a, 16, 0);
    check("size_16_ok", fd == 3, fd);
    close((int)fd);
    fd = ll_create(&b.a, sizeof b, 0);
    check("size_32_zero_tail_ok", fd == 3, fd);
    close((int)fd);
    struct k_ll_ruleset_attr net = { 0, LANDLOCK_ACCESS_NET_BIND_TCP | LANDLOCK_ACCESS_NET_CONNECT_TCP, 0 };
    fd = ll_create(&net, sizeof net, 0);
    check("net_only", fd == 3, fd);
    close((int)fd);
    struct k_ll_ruleset_attr sc = { 0, 0, LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET | LANDLOCK_SCOPE_SIGNAL };
    fd = ll_create(&sc, sizeof sc, 0);
    check("scoped_only", fd == 3, fd);
    close((int)fd);
    struct k_ll_ruleset_attr all = { (1UL << 16) - 1, 3, 3 };
    fd = ll_create(&all, sizeof all, 0);
    check("all_known_bits", fd == 3, fd);
    close((int)fd);
    return finish();
}
