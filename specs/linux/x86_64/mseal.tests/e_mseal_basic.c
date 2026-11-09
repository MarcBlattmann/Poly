/* mseal(2) seals a mapped range so its mappings can no longer be changed:
 * afterwards munmap, mprotect, mremap and mmap(MAP_FIXED) over any part of
 * it fail with EPERM (also when the request covers sealed and unsealed
 * pages, in which case nothing changes). The memory itself stays usable
 * (reads and writes as allowed by its protection). Sealing again is fine.
 * Neighbouring unsealed pages are not affected. A forked child inherits
 * the seal.
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
    char *m = (char *)map_anon(4 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE);
    check("mapped", !BAD_PTR(m), (i64)m);
    i64 r = syscall3(SYS_mseal, m, 4096, 0);
    check("seal_first_page", r == 0, r);
    m[0] = 'a';
    check("still_writable", m[0] == 'a', 0);
    r = syscall3(SYS_mprotect, m, 4096, PROT_READ);
    check("mprotect_eperm", r == -EPERM, r);
    r = syscall2(SYS_munmap, m, 4096);
    check("munmap_eperm", r == -EPERM, r);
    r = syscall5(SYS_mremap, m, 4096, 8192, MREMAP_MAYMOVE, 0);
    check("mremap_eperm", r == -EPERM, r);
    r = syscall6(SYS_mmap, (i64)m, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    check("mmap_fixed_eperm", r == -EPERM, r);
    r = syscall2(SYS_munmap, m, 4 * 4096);
    check("munmap_partly_sealed_eperm", r == -EPERM, r);
    m[4096] = 'b';
    check("neighbour_untouched_by_failed_munmap", m[4096] == 'b', 0);
    r = syscall3(SYS_mprotect, m + 4096, 4096, PROT_READ);
    check("neighbour_mprotect_ok", r == 0, r);
    r = syscall3(SYS_mseal, m, 4096, 0);
    check("seal_again_ok", r == 0, r);
    r = syscall2(SYS_munmap, m + 2 * 4096, 2 * 4096);
    check("neighbour_munmap_ok", r == 0, r);
    i64 pid = fork_sync();
    if (pid == 0) exit_group(syscall2(SYS_munmap, m, 4096) == -EPERM ? 0 : 1);
    int st = wait_exit(pid);
    check("child_inherits_seal", st == 0, st);
    return finish();
}
