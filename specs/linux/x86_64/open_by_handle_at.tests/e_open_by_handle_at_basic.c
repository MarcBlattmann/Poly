/* open_by_handle_at(2) opens the file a handle from name_to_handle_at
 * names, on the file system of mount_fd (any descriptor on that file
 * system), with open flags (O_RDONLY, O_RDWR, O_PATH, O_CLOEXEC; O_CREAT
 * is ignored). The result is a new descriptor (lowest free) for the same
 * file, also after the file was renamed. Needs CAP_DAC_READ_SEARCH.
 * poly-diff: needs=root (CAP_DAC_READ_SEARCH)
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

#define F "/dev/shm/e_open_by_handle_at_basic"
#define G "/dev/shm/e_open_by_handle_at_basic.renamed"

int test_main(void) {
    make_file(F, "handle!", 0644);
    unlink_path(G);
    struct k_file_handle h;
    int mid;
    memset(&h, 0, sizeof h);
    h.handle_bytes = MAX_HANDLE_SZ;
    check("handle", syscall5(SYS_name_to_handle_at, AT_FDCWD, F, &h, &mid, 0) == 0, 0);
    i64 mfd = open3("/dev/shm", O_RDONLY | O_DIRECTORY, 0);
    i64 fd = syscall3(SYS_open_by_handle_at, mfd, &h, O_RDONLY);
    check("open_ro", fd == 4, fd);
    char buf[16];
    memset(buf, 0, sizeof buf);
    check("read", read((int)fd, buf, sizeof buf) == 7 && mem_eq(buf, "handle!", 7), 0);
    check("ro_flags", syscall2(SYS_fcntl, fd, F_GETFL) == 0100000 /* O_LARGEFILE */, syscall2(SYS_fcntl, fd, F_GETFL));
    close((int)fd);
    fd = syscall3(SYS_open_by_handle_at, mfd, &h, O_RDWR | O_CLOEXEC);
    check("open_rdwr_cloexec", fd == 4 && syscall2(SYS_fcntl, fd, F_GETFD) == FD_CLOEXEC, fd);
    check("write", syscall4(18 /* pwrite64 */, fd, "H", 1, 0) == 1, 0);
    close((int)fd);
    fd = syscall3(SYS_open_by_handle_at, mfd, &h, O_PATH);
    check("open_path", fd == 4 && (syscall2(SYS_fcntl, fd, F_GETFL) & O_PATH), fd);
    close((int)fd);
    fd = syscall3(SYS_open_by_handle_at, mfd, &h, O_RDONLY | O_CREAT);
    check("o_creat_ignored", fd == 4, fd);
    close((int)fd);
    i64 other = open3(F, O_RDONLY, 0);
    fd = syscall3(SYS_open_by_handle_at, other, &h, O_RDONLY);
    check("mount_fd_any_file_on_fs", fd >= 0, fd);
    close((int)fd);
    close((int)other);
    check("rename", syscall4(264, AT_FDCWD, F, AT_FDCWD, G) == 0, 0);
    fd = syscall3(SYS_open_by_handle_at, mfd, &h, O_RDONLY);
    memset(buf, 0, sizeof buf);
    check("after_rename", fd >= 0 && read((int)fd, buf, sizeof buf) == 7 && buf[0] == 'H', fd);
    close((int)fd);
    close((int)mfd);
    unlink_path(G);
    return finish();
}
