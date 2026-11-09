/* name_to_handle_at(2) returns an opaque file handle for (dirfd, path) in
 * struct file_handle {u32 handle_bytes; s32 handle_type; f_handle[]} and
 * the mount id in *mount_id. handle_bytes is the buffer size on entry and
 * the handle's size on return; the same file always gives the same handle,
 * different files different handles. A final symlink is not followed
 * unless AT_SYMLINK_FOLLOW; AT_EMPTY_PATH with "" names dirfd. With
 * AT_HANDLE_MNT_ID_UNIQUE mount_id points at a u64 that receives the
 * unique mount id (the one statx STATX_MNT_ID_UNIQUE reports).
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

#define F "/dev/shm/e_name_to_handle_at_basic"
#define G "/dev/shm/e_name_to_handle_at_basic.g"
#define L "/dev/shm/e_name_to_handle_at_basic.l"

static int same(struct k_file_handle *a, struct k_file_handle *b) {
    return a->handle_bytes == b->handle_bytes && a->handle_type == b->handle_type &&
           mem_eq(a->f_handle, b->f_handle, a->handle_bytes);
}

int test_main(void) {
    make_file(F, "abc", 0644);
    make_file(G, "xyz", 0644);
    unlink_path(L);
    syscall3(SYS_symlinkat, F, AT_FDCWD, L);
    struct k_file_handle h1, h2, hg, hl, he;
    int mid = -1, mid2 = -1;
    memset(&h1, 0, sizeof h1);
    h1.handle_bytes = MAX_HANDLE_SZ;
    i64 r = syscall5(SYS_name_to_handle_at, AT_FDCWD, F, &h1, &mid, 0);
    check("ok", r == 0, r);
    check("bytes_sane", h1.handle_bytes > 0 && h1.handle_bytes <= MAX_HANDLE_SZ, h1.handle_bytes);
    check("mount_id_set", mid >= 0, mid);
    memset(&h2, 0, sizeof h2);
    h2.handle_bytes = MAX_HANDLE_SZ;
    syscall5(SYS_name_to_handle_at, AT_FDCWD, F, &h2, &mid2, 0);
    check("stable", same(&h1, &h2) && mid == mid2, 0);
    memset(&hg, 0, sizeof hg);
    hg.handle_bytes = MAX_HANDLE_SZ;
    syscall5(SYS_name_to_handle_at, AT_FDCWD, G, &hg, &mid2, 0);
    check("other_file_differs", !same(&h1, &hg) && mid2 == mid, 0);

    memset(&hl, 0, sizeof hl);
    hl.handle_bytes = MAX_HANDLE_SZ;
    syscall5(SYS_name_to_handle_at, AT_FDCWD, L, &hl, &mid2, 0);
    check("symlink_not_followed", !same(&h1, &hl), 0);
    memset(&hl, 0, sizeof hl);
    hl.handle_bytes = MAX_HANDLE_SZ;
    syscall5(SYS_name_to_handle_at, AT_FDCWD, L, &hl, &mid2, AT_SYMLINK_FOLLOW);
    check("symlink_followed", same(&h1, &hl), 0);
    i64 fd = open3(F, O_RDONLY, 0);
    memset(&he, 0, sizeof he);
    he.handle_bytes = MAX_HANDLE_SZ;
    r = syscall5(SYS_name_to_handle_at, fd, "", &he, &mid2, AT_EMPTY_PATH);
    check("empty_path", r == 0 && same(&h1, &he), r);
    i64 dfd = open3("/dev/shm", O_RDONLY | O_DIRECTORY, 0);
    memset(&he, 0, sizeof he);
    he.handle_bytes = MAX_HANDLE_SZ;
    r = syscall5(SYS_name_to_handle_at, dfd, "e_name_to_handle_at_basic", &he, &mid2, 0);
    check("relative", r == 0 && same(&h1, &he), r);

    struct {
        unsigned mask, blksize;
        u64 attributes;
        unsigned nlink, uid, gid;
        unsigned short mode, pad1;
        u64 ino, size, blocks, attributes_mask;
        u64 times[8];
        unsigned rdev_major, rdev_minor, dev_major, dev_minor;
        u64 mnt_id;
        u64 rest[14];
    } sx;
    memset(&sx, 0, sizeof sx);
    syscall5(SYS_statx, AT_FDCWD, F, 0, 0x4000 /* STATX_MNT_ID_UNIQUE */, &sx);
    u64 umid = 0;
    memset(&he, 0, sizeof he);
    he.handle_bytes = MAX_HANDLE_SZ;
    r = syscall5(SYS_name_to_handle_at, AT_FDCWD, F, &he, &umid, AT_HANDLE_MNT_ID_UNIQUE);
    check("unique_mount_id", r == 0 && umid == sx.mnt_id && umid != 0, r);
    close((int)fd);
    close((int)dfd);
    unlink_path(L);
    unlink_path(F);
    unlink_path(G);
    return finish();
}
