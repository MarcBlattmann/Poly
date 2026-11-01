/* open_by_handle_at(2) errors: handle_bytes 0 or above 128 EINVAL; an
 * unreadable handle EFAULT; a closed mount_fd EBADF; a mount_fd on another
 * file system (procfs) or AT_FDCWD outside it, or a handle whose bytes were
 * corrupted, ESTALE; a handle of a file that was deleted (and is no longer
 * open anywhere) ESTALE. A caller without CAP_DAC_READ_SEARCH gets EPERM
 * (after the handle_bytes check).
 * poly-diff: needs=root (CAP_DAC_READ_SEARCH, switching to another user)
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

#define F "/dev/shm/e_open_by_handle_at_errors"

int test_main(void) {
    make_file(F, "x", 0644);
    struct k_file_handle h, b;
    int mid;
    memset(&h, 0, sizeof h);
    h.handle_bytes = MAX_HANDLE_SZ;
    syscall5(SYS_name_to_handle_at, AT_FDCWD, F, &h, &mid, 0);
    i64 mfd = open3("/dev/shm", O_RDONLY | O_DIRECTORY, 0);
    i64 r;
    b = h;
    b.handle_bytes = 0;
    r = syscall3(SYS_open_by_handle_at, mfd, &b, O_RDONLY);
    check("bytes_0_einval", r == -EINVAL, r);
    b.handle_bytes = MAX_HANDLE_SZ + 1;
    r = syscall3(SYS_open_by_handle_at, mfd, &b, O_RDONLY);
    check("bytes_129_einval", r == -EINVAL, r);
    r = syscall3(SYS_open_by_handle_at, mfd, (void *)8, O_RDONLY);
    check("handle_efault", r == -EFAULT, r);
    r = syscall3(SYS_open_by_handle_at, 99, &h, O_RDONLY);
    check("closed_mount_fd_ebadf", r == -EBADF, r);
    i64 pf = open3("/proc", O_RDONLY | O_DIRECTORY, 0);
    r = syscall3(SYS_open_by_handle_at, pf, &h, O_RDONLY);
    check("other_fs_estale", r == -ESTALE, r);
    close((int)pf);
    b = h;
    for (unsigned i = 0; i < b.handle_bytes; i++) b.f_handle[i] ^= 0xff;
    r = syscall3(SYS_open_by_handle_at, mfd, &b, O_RDONLY);
    check("corrupt_estale", r == -ESTALE, r);

    i64 pid = fork_sync();
    if (pid == 0) {
        if (!drop_to_user()) exit_group(100);
        int bad = 0;
        if (syscall3(SYS_open_by_handle_at, mfd, &h, O_RDONLY) != -EPERM) bad |= 1;
        struct k_file_handle z = h;
        z.handle_bytes = 0;
        if (syscall3(SYS_open_by_handle_at, mfd, &z, O_RDONLY) != -EINVAL) bad |= 2;
        exit_group(bad);
    }
    int st = wait_exit(pid);
    check("user_eperm", st == 0, st);

    check("unlink", unlink_path(F) == 0, 0);
    r = syscall3(SYS_open_by_handle_at, mfd, &h, O_RDONLY);
    check("deleted_estale", r == -ESTALE, r);
    close((int)mfd);
    return finish();
}
