/* memfd_secret(2) creates an anonymous file whose memory is reachable
 * only through mmap(MAP_SHARED) of it (removed from the kernel's direct
 * map). It returns a descriptor (lowest free; FD_CLOEXEC only with
 * O_CLOEXEC; F_GETFL O_RDWR|O_LARGEFILE); fstat shows a regular file, mode
 * 0600, size 0. The size is set once with ftruncate; the memory then reads
 * as zeros and is shared by every mapping and across fork. Errors: flags
 * other than O_CLOEXEC EINVAL.
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

int test_main(void) {
    i64 r = syscall1(SYS_memfd_secret, 1);
    check("bad_flags_einval", r == -EINVAL, r);
    r = syscall1(SYS_memfd_secret, O_NONBLOCK);
    check("nonblock_einval", r == -EINVAL, r);
    i64 fd = syscall1(SYS_memfd_secret, 0);
    check("fd", fd == 3, fd);
    check("no_cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == 0, 0);
    check("rdwr", syscall2(SYS_fcntl, fd, F_GETFL) == (O_RDWR | 0100000), syscall2(SYS_fcntl, fd, F_GETFL));
    i64 c = syscall1(SYS_memfd_secret, O_CLOEXEC);
    check("cloexec", c == 4 && syscall2(SYS_fcntl, c, F_GETFD) == FD_CLOEXEC, c);
    close((int)c);
    struct {
        u64 dev, ino, nlink;
        unsigned mode, uid, gid, pad;
        u64 rdev;
        i64 size;
        char rest[80];
    } st;
    memset(&st, 0, sizeof st);
    syscall2(5 /* fstat */, fd, &st);
    check("mode", st.mode == (0100000 | 0600), st.mode);
    check("size_0", st.size == 0, st.size);
    check("ftruncate", syscall2(SYS_ftruncate, fd, 8192) == 0, 0);
    syscall2(5, fd, &st);
    check("size_8192", st.size == 8192, st.size);
    char *p = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap_shared", !BAD_PTR(p), (i64)p);
    check("zero_filled", p[0] == 0 && p[8191] == 0, p[0]);
    p[100] = 42;
    char *q = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("second_mapping_shares", !BAD_PTR(q) && q[100] == 42, 0);
    i64 pid = fork_sync();
    if (pid == 0) {
        int ok = p[100] == 42;
        p[200] = 7;
        exit_group(ok ? 0 : 1);
    }
    int ws = wait_exit(pid);
    check("child_sees_and_writes", ws == 0 && q[200] == 7, ws);
    close((int)fd);
    check("mapping_outlives_fd", p[100] == 42, 0);
    return finish();
}
