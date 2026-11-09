/* cachestat(2) errors: flags != 0 EINVAL; a closed fd or an O_PATH fd
 * EBADF; an unreadable range or unwritable result EFAULT. Descriptors
 * without page cache of their own (a pipe, a socket, a directory,
 * /dev/null) succeed and report all zeros.
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

#define F "/dev/shm/e_cachestat_errors"

struct k_cachestat {
    u64 nr_cache, nr_dirty, nr_writeback, nr_evicted, nr_recently_evicted;
};
struct k_range {
    u64 off, len;
};

static int zeros(struct k_cachestat *c) {
    return c->nr_cache == 0 && c->nr_dirty == 0 && c->nr_writeback == 0 && c->nr_evicted == 0 &&
           c->nr_recently_evicted == 0;
}

int test_main(void) {
    unlink_path(F);
    i64 fd = open3(F, O_CREAT | O_RDWR | O_TRUNC, 0644);
    write((int)fd, "x", 1);
    struct k_cachestat cs;
    struct k_range rg = { 0, 0 };
    i64 r;
    r = syscall4(SYS_cachestat, fd, &rg, &cs, 1);
    check("flags_einval", r == -EINVAL, r);
    r = syscall4(SYS_cachestat, 99, &rg, &cs, 0);
    check("closed_ebadf", r == -EBADF, r);
    i64 op = open3(F, O_PATH, 0);
    r = syscall4(SYS_cachestat, op, &rg, &cs, 0);
    check("o_path_ebadf", r == -EBADF, r);
    r = syscall4(SYS_cachestat, fd, (void *)8, &cs, 0);
    check("range_efault", r == -EFAULT, r);
    r = syscall4(SYS_cachestat, fd, &rg, (void *)8, 0);
    check("result_efault", r == -EFAULT, r);
    int p[2];
    syscall2(SYS_pipe2, p, 0);
    memset(&cs, 0xee, sizeof cs);
    r = syscall4(SYS_cachestat, p[0], &rg, &cs, 0);
    check("pipe_zeros", r == 0 && zeros(&cs), r);
    i64 s = syscall3(SYS_socket, AF_UNIX, SOCK_STREAM, 0);
    memset(&cs, 0xee, sizeof cs);
    r = syscall4(SYS_cachestat, s, &rg, &cs, 0);
    check("socket_zeros", r == 0 && zeros(&cs), r);
    i64 d = open3("/dev/shm", O_RDONLY | O_DIRECTORY, 0);
    memset(&cs, 0xee, sizeof cs);
    r = syscall4(SYS_cachestat, d, &rg, &cs, 0);
    check("dir_zeros", r == 0 && zeros(&cs), r);
    i64 n = open3("/dev/null", O_RDONLY, 0);
    memset(&cs, 0xee, sizeof cs);
    r = syscall4(SYS_cachestat, n, &rg, &cs, 0);
    check("devnull_zeros", r == 0 && zeros(&cs), r);
    close((int)n);
    close((int)d);
    close((int)s);
    close(p[0]);
    close(p[1]);
    close((int)op);
    close((int)fd);
    unlink_path(F);
    return finish();
}
