/* fanotify_mark(2) on a directory: with FAN_EVENT_ON_CHILD the events of
 * files directly in it are reported (fd names the child); without it only
 * the directory's own events, and opening the directory itself needs
 * FAN_ONDIR. With dfd a file descriptor and path NULL the mark goes on
 * dfd's file. FAN_MARK_IGNORED_MASK on a child suppresses its events. A
 * FID group with FAN_CREATE|FAN_DELETE on the directory reports entry
 * creation and removal (fd FAN_NOFD).
 * poly-diff: needs=root (CAP_SYS_ADMIN)
 * poly-diff: syscalls=names (events carry the pid and inode numbers)
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

#define D "/dev/shm/e_fanotify_mark_dir"

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/g", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR);
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    make_file(D "/f", "", 0644);
    make_file(D "/g", "", 0644);
    i64 fan = fan_init(FAN_CLOEXEC | FAN_NONBLOCK, O_RDONLY);
    static char buf[1024];
    struct k_fan_meta *m = (struct k_fan_meta *)buf;
    check("mark_dir_children", fan_mark((int)fan, FAN_MARK_ADD, FAN_OPEN | FAN_EVENT_ON_CHILD, AT_FDCWD, D) == 0, 0);
    touch_open(D "/f", O_RDONLY);
    i64 n = read((int)fan, buf, sizeof buf);
    check("child_event", n == 24 && m->mask == FAN_OPEN, (i64)m->mask);
    i64 dfd = open3(D, O_RDONLY | O_DIRECTORY, 0);
    struct k_file_handle h1, h2;
    int mid;
    memset(&h1, 0, sizeof h1);
    memset(&h2, 0, sizeof h2);
    h1.handle_bytes = h2.handle_bytes = MAX_HANDLE_SZ;
    syscall5(SYS_name_to_handle_at, m->fd, "", &h1, &mid, AT_EMPTY_PATH);
    syscall5(SYS_name_to_handle_at, AT_FDCWD, D "/f", &h2, &mid, 0);
    check("event_fd_is_child", mem_eq(h1.f_handle, h2.f_handle, h2.handle_bytes), 0);
    close(m->fd);
    close((int)dfd);
    check("dir_open_not_reported_without_ondir", read((int)fan, buf, sizeof buf) == -EAGAIN, 0);
    fan_mark((int)fan, FAN_MARK_ADD, FAN_OPEN | FAN_ONDIR | FAN_EVENT_ON_CHILD, AT_FDCWD, D);
    touch_open(D, O_RDONLY | O_DIRECTORY);
    n = read((int)fan, buf, sizeof buf);
    /* A descriptor-reporting group does not put FAN_ONDIR in the event. */
    check("dir_open_with_ondir", n == 24 && m->mask == FAN_OPEN, (i64)m->mask);
    close(m->fd);

    i64 gfd = open3(D "/g", O_RDONLY, 0);
    check("ignore_g", fan_mark((int)fan, FAN_MARK_ADD | FAN_MARK_IGNORED_MASK, FAN_OPEN, (int)gfd, 0) == 0, 0);
    close((int)gfd);
    while (read((int)fan, buf, sizeof buf) > 0) close(m->fd);
    touch_open(D "/g", O_RDONLY);
    check("g_ignored", read((int)fan, buf, sizeof buf) == -EAGAIN, 0);
    touch_open(D "/f", O_RDONLY);
    n = read((int)fan, buf, sizeof buf);
    check("f_still_reported", n == 24 && m->mask == FAN_OPEN, n);
    close(m->fd);
    close((int)fan);

    i64 ff = fan_init(FAN_REPORT_FID | FAN_CLOEXEC | FAN_NONBLOCK, 0);
    check("fid_mark_entries", fan_mark((int)ff, FAN_MARK_ADD, FAN_CREATE | FAN_DELETE, AT_FDCWD, D) == 0, 0);
    make_file(D "/h", "", 0644);
    unlink_path(D "/h");
    n = read((int)ff, buf, sizeof buf);
    check("entry_events", n > 24 && m->fd == FAN_NOFD && (m->mask & (FAN_CREATE | FAN_DELETE)) != 0, (i64)m->mask);
    u64 all = 0;
    for (i64 o = 0; o < n;) {
        struct k_fan_meta *e = (struct k_fan_meta *)(buf + o);
        all |= e->mask;
        o += e->event_len;
    }
    check("create_and_delete_seen", all == (FAN_CREATE | FAN_DELETE), (i64)all);
    close((int)ff);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/f", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/g", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR);
    return finish();
}
