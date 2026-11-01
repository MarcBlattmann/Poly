/* fanotify_init(2) creates a fanotify group and returns its descriptor
 * (lowest free; F_GETFL shows O_RDWR; FAN_CLOEXEC sets FD_CLOEXEC,
 * FAN_NONBLOCK sets O_NONBLOCK). event_f_flags are the open flags of the descriptors handed
 * out with events. An empty group reads EAGAIN when non-blocking; write is
 * EINVAL. Errors: unknown flags EINVAL; more than one class EINVAL; bad
 * event_f_flags (O_CREAT) EINVAL; FAN_REPORT_NAME without
 * FAN_REPORT_DIR_FID EINVAL; FAN_REPORT_FID with a permission class
 * EINVAL.
 * poly-diff: needs=root (CAP_SYS_ADMIN for a classic group)
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
/* linux/fanotify.h */
#define FAN_CLOEXEC 0x01
#define FAN_NONBLOCK 0x02
#define FAN_CLASS_NOTIF 0x00
#define FAN_CLASS_CONTENT 0x04
#define FAN_CLASS_PRE_CONTENT 0x08
#define FAN_UNLIMITED_QUEUE 0x10
#define FAN_UNLIMITED_MARKS 0x20
#define FAN_REPORT_FID 0x200
#define FAN_REPORT_DIR_FID 0x400
#define FAN_REPORT_NAME 0x800
#define FAN_MARK_ADD 0x01
#define FAN_MARK_REMOVE 0x02
#define FAN_MARK_DONT_FOLLOW 0x04
#define FAN_MARK_ONLYDIR 0x08
#define FAN_MARK_MOUNT 0x10
#define FAN_MARK_IGNORED_MASK 0x20
#define FAN_MARK_FLUSH 0x80
#define FAN_MARK_FILESYSTEM 0x100
#define FAN_ACCESS 0x01
#define FAN_MODIFY 0x02
#define FAN_ATTRIB 0x04
#define FAN_CLOSE_WRITE 0x08
#define FAN_CLOSE_NOWRITE 0x10
#define FAN_OPEN 0x20
#define FAN_CREATE 0x100
#define FAN_DELETE 0x200
#define FAN_OPEN_PERM 0x10000
#define FAN_EVENT_ON_CHILD 0x08000000
#define FAN_ONDIR 0x40000000
#define FANOTIFY_METADATA_VERSION 3
#define FAN_EVENT_INFO_TYPE_FID 1
#define FAN_NOFD (-1)
/* struct fanotify_event_metadata (24 bytes). */
struct k_fan_meta {
    unsigned event_len;
    unsigned char vers, reserved;
    unsigned short metadata_len;
    u64 mask;
    int fd;
    int pid;
};
static __attribute__((unused)) i64 fan_init(unsigned flags, unsigned evflags) {
    return syscall2(SYS_fanotify_init, flags, evflags);
}
static __attribute__((unused)) i64 fan_mark(int fd, unsigned flags, u64 mask, int dfd, const char *path) {
    return syscall5(SYS_fanotify_mark, fd, flags, mask, dfd, path);
}
static __attribute__((unused)) void touch_open(const char *path, i64 flags) {
    i64 fd = open3(path, flags, 0);
    if (fd >= 0) close((int)fd);
}

int test_main(void) {
    i64 r;
    r = fan_init(1U << 30, 0);
    check("bad_flags_einval", r == -EINVAL, r);
    r = fan_init(FAN_CLASS_CONTENT | FAN_CLASS_PRE_CONTENT, 0);
    check("two_classes_einval", r == -EINVAL, r);
    r = fan_init(0, O_CREAT);
    check("bad_event_flags_einval", r == -EINVAL, r);
    r = fan_init(FAN_REPORT_NAME, 0);
    check("name_without_dir_fid_einval", r == -EINVAL, r);
    r = fan_init(FAN_REPORT_FID | FAN_CLASS_CONTENT, 0);
    check("fid_with_content_einval", r == -EINVAL, r);

    i64 fd = fan_init(FAN_CLOEXEC | FAN_NONBLOCK, O_RDONLY);
    check("fd", fd == 3, fd);
    check("cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == FD_CLOEXEC, syscall2(SYS_fcntl, fd, F_GETFD));
    check("status_flags", syscall2(SYS_fcntl, fd, F_GETFL) == (O_RDWR | O_NONBLOCK), syscall2(SYS_fcntl, fd, F_GETFL));
    char buf[64];
    r = read((int)fd, buf, sizeof buf);
    check("empty_eagain", r == -EAGAIN, r);
    r = write((int)fd, "x", 1);
    check("write_einval", r == -EINVAL, r);
    close((int)fd);
    fd = fan_init(0, O_RDONLY);
    check("plain", fd == 3 && syscall2(SYS_fcntl, fd, F_GETFD) == 0 && syscall2(SYS_fcntl, fd, F_GETFL) == O_RDWR, fd);
    close((int)fd);
    fd = fan_init(FAN_CLASS_CONTENT | FAN_UNLIMITED_QUEUE | FAN_UNLIMITED_MARKS, O_RDONLY);
    check("content_unlimited", fd >= 0, fd);
    close((int)fd);
    fd = fan_init(FAN_REPORT_FID | FAN_REPORT_DIR_FID | FAN_REPORT_NAME, 0);
    check("fid_dir_fid_name", fd >= 0, fd);
    close((int)fd);
    return finish();
}
