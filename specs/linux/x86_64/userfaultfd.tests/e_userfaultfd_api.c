/* userfaultfd(2) returns a descriptor (O_CLOEXEC and O_NONBLOCK as asked;
 * UFFD_USER_MODE_ONLY limits it to faults from user mode and is what an
 * unprivileged caller needs when vm.unprivileged_userfaultfd is 0). Before
 * anything else the UFFDIO_API ioctl must be done once with api =
 * UFFD_API (0xAA): it returns the supported features and the ioctls
 * available (at least UFFDIO_API, REGISTER, UNREGISTER). Errors:
 * UFFDIO_REGISTER or read before UFFDIO_API EINVAL; a wrong api EINVAL
 * (the handshake can then be retried); unknown requested feature bits
 * EINVAL with the features field zeroed; UFFDIO_API a second time EINVAL.
 * read with nothing pending: EAGAIN when non-blocking; a buffer smaller
 * than one message (32 bytes) EINVAL.
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
/* linux/userfaultfd.h */
#define UFFD_API 0xAAUL
#define UFFD_USER_MODE_ONLY 1
#define UFFD_FEATURE_SIGBUS (1UL << 7)
#define UFFD_FEATURE_THREAD_ID (1UL << 8)
#define UFFD_EVENT_PAGEFAULT 0x12
#define UFFD_PAGEFAULT_FLAG_WRITE (1UL << 0)
#define UFFDIO_REGISTER_MODE_MISSING (1UL << 0)
#define UFFDIO_REGISTER_MODE_WP (1UL << 1)
/* ioctl numbers: _IOWR(0xAA, nr, struct) */
#define UFFDIO_API 0xc018aa3fUL
#define UFFDIO_REGISTER 0xc020aa00UL
#define UFFDIO_UNREGISTER 0x8010aa01UL
#define UFFDIO_WAKE 0x8010aa02UL
#define UFFDIO_COPY 0xc028aa03UL
#define UFFDIO_ZEROPAGE 0xc020aa04UL
/* bits of the ioctls fields: 1 << _UFFDIO_* (nr) */
#define B_UFFDIO_REGISTER (1UL << 0x00)
#define B_UFFDIO_UNREGISTER (1UL << 0x01)
#define B_UFFDIO_WAKE (1UL << 0x02)
#define B_UFFDIO_COPY (1UL << 0x03)
#define B_UFFDIO_ZEROPAGE (1UL << 0x04)
#define B_UFFDIO_API (1UL << 0x3F)
struct k_uffdio_api {
    u64 api, features, ioctls;
};
struct k_uffdio_range {
    u64 start, len;
};
struct k_uffdio_register {
    struct k_uffdio_range range;
    u64 mode, ioctls;
};
struct k_uffdio_copy {
    u64 dst, src, len, mode;
    i64 copy;
};
struct k_uffdio_zeropage {
    struct k_uffdio_range range;
    u64 mode;
    i64 zeropage;
};
/* struct uffd_msg (32 bytes), pagefault view. */
struct k_uffd_msg {
    unsigned char event, reserved1;
    unsigned short reserved2;
    unsigned reserved3;
    u64 flags;
    u64 address;
    unsigned ptid;
    unsigned pad;
};
/* clone(flags, stack) running fn(arg) on the new stack in the child, which
 * then exits (60) with fn's result. Returns the child's pid or -errno. */
long k_clone_fn(u64 flags, void *stack_top, int (*fn)(void *), void *arg);
__asm__(".text\n"
        ".global k_clone_fn\n"
        "k_clone_fn:\n"
        "    and $-16, %rsi\n"
        "    sub $16, %rsi\n"
        "    mov %rdx, 0(%rsi)\n"
        "    mov %rcx, 8(%rsi)\n"
        "    xor %edx, %edx\n"
        "    xor %r10d, %r10d\n"
        "    xor %r8d, %r8d\n"
        "    mov $56, %eax\n"
        "    syscall\n"
        "    test %rax, %rax\n"
        "    jnz 1f\n"
        "    xor %ebp, %ebp\n"
        "    pop %rax\n"
        "    pop %rdi\n"
        "    call *%rax\n"
        "    mov %eax, %edi\n"
        "    mov $60, %eax\n"
        "    syscall\n"
        "    hlt\n"
        "1:  ret\n");
static char k_child_stack[65536] __attribute__((aligned(16)));
/* A child process sharing this address space (CLONE_VM), SIGCHLD at exit. */
static __attribute__((unused)) i64 a_clone_vm(volatile int *unused, int (*fn)(void *)) {
    (void)unused;
    return k_clone_fn(CLONE_VM | SIGCHLD, k_child_stack + sizeof k_child_stack, fn, 0);
}
static __attribute__((unused)) i64 uffd_open(unsigned flags) { return syscall1(SYS_userfaultfd, flags); }
static __attribute__((unused)) i64 uffd_ioctl(int fd, u64 req, void *arg) { return syscall3(SYS_ioctl, fd, req, arg); }
/* A userfaultfd that works for any user (UFFD_USER_MODE_ONLY), API done. */
static __attribute__((unused)) i64 uffd_ready(unsigned flags, u64 features) {
    i64 fd = uffd_open(flags | UFFD_USER_MODE_ONLY);
    if (fd < 0) return fd;
    struct k_uffdio_api api = { UFFD_API, features, 0 };
    i64 r = uffd_ioctl((int)fd, UFFDIO_API, &api);
    if (r < 0) {
        close((int)fd);
        return r;
    }
    return fd;
}

int test_main(void) {
    i64 fd = uffd_open(O_CLOEXEC | O_NONBLOCK | UFFD_USER_MODE_ONLY);
    check("fd", fd == 3, fd);
    check("cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == FD_CLOEXEC, 0);
    check("nonblock", (syscall2(SYS_fcntl, fd, F_GETFL) & O_NONBLOCK) != 0, syscall2(SYS_fcntl, fd, F_GETFL));
    struct k_uffdio_register reg;
    memset(&reg, 0, sizeof reg);
    i64 r = uffd_ioctl((int)fd, UFFDIO_REGISTER, &reg);
    check("register_before_api_einval", r == -EINVAL, r);
    struct k_uffd_msg msg;
    r = read((int)fd, &msg, sizeof msg);
    check("read_before_api_einval", r == -EINVAL, r);

    struct k_uffdio_api api = { 0x55, 0, 0 };
    r = uffd_ioctl((int)fd, UFFDIO_API, &api);
    check("wrong_api_einval", r == -EINVAL, r);
    api.api = UFFD_API;
    api.features = 0;
    r = uffd_ioctl((int)fd, UFFDIO_API, &api);
    check("retry_after_wrong_api_ok", r == 0, r);
    u64 need = B_UFFDIO_API | B_UFFDIO_REGISTER | B_UFFDIO_UNREGISTER;
    check("ioctls", (api.ioctls & need) == need, (i64)api.ioctls);
    check("feature_sigbus_offered", (api.features & UFFD_FEATURE_SIGBUS) != 0, (i64)api.features);
    api.features = 0;
    r = uffd_ioctl((int)fd, UFFDIO_API, &api);
    check("api_twice_einval", r == -EINVAL, r);
    r = read((int)fd, &msg, sizeof msg);
    check("nothing_eagain", r == -EAGAIN, r);
    r = read((int)fd, &msg, 8);
    check("small_read_einval", r == -EINVAL, r);
    close((int)fd);

    i64 f2 = uffd_open(O_CLOEXEC | UFFD_USER_MODE_ONLY);
    struct k_uffdio_api a2 = { UFFD_API, 1UL << 62, 0 };
    r = uffd_ioctl((int)f2, UFFDIO_API, &a2);
    check("unknown_feature_einval", r == -EINVAL, r);
    check("features_zeroed", a2.features == 0, (i64)a2.features);
    close((int)f2);
    return finish();
}
