/* clock_settime(2) on CLOCK_REALTIME: the timespec is read (EFAULT when it
 * cannot be) and checked (EINVAL for tv_nsec outside [0, 999999999], a
 * negative tv_sec, or a tv_sec too large for the clock) before the
 * permission check, so these errors are the same for root and other users.
 * A valid time without CAP_SYS_TIME is EPERM (checked in a child that
 * drops to uid/gid 65534 when the test runs as root).
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_gettimeofday 96
#define SYS_syslog 103
#define SYS_ustat 136
#define SYS_sysfs 139
#define SYS_adjtimex 159
#define SYS_acct 163
#define SYS_settimeofday 164
#define SYS_clock_settime 227
#define SYS_newfstatat 262
#define SYS_unshare 272
#define SYS_clock_adjtime 305
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EISDIR 21
/* linux/time.h (clock ids) */
#define CLOCK_PROCESS_CPUTIME_ID 2
#define CLOCK_THREAD_CPUTIME_ID 3
#define CLOCK_MONOTONIC_RAW 4
#define CLOCK_REALTIME_COARSE 5
#define CLOCK_MONOTONIC_COARSE 6
#define CLOCK_BOOTTIME 7
#define CLOCK_REALTIME_ALARM 8
#define CLOCK_BOOTTIME_ALARM 9
#define CLOCK_SGI_CYCLE 10
#define CLOCK_TAI 11
/* A CPU clock id of the calling process (pid 0) or thread, as clock_getcpuclockid
 * and pthread_getcpuclockid build them (time(7)): ~0 << 3 | which. */
#define CPUCLOCK_SELF_PROCESS_SCHED (-6)
#define CPUCLOCK_SELF_THREAD_SCHED (-2)
/* linux/sched.h */
#define CLONE_NEWPID 0x20000000
#define CLONE_VFORK 0x00004000
/* asm-generic/fcntl.h */
#define AT_SYMLINK_NOFOLLOW 0x100

/* struct timezone, linux/time.h */
struct k_timezone {
    int tz_minuteswest;
    int tz_dsttime;
};

/* struct __kernel_timex, linux/timex.h (x86-64: 208 bytes). */
struct k_timex {
    unsigned modes;
    int pad0;
    i64 offset, freq, maxerror, esterror;
    int status;
    int pad1;
    i64 constant, precision, tolerance;
    struct k_timeval time;
    i64 tick, ppsfreq, jitter;
    int shift;
    int pad2;
    i64 stabil, jitcnt, calcnt, errcnt, stbcnt;
    int tai;
    int pad3[11];
};
/* linux/timex.h: modes */
#define ADJ_OFFSET 0x0001
#define ADJ_FREQUENCY 0x0002
#define ADJ_MAXERROR 0x0004
#define ADJ_ESTERROR 0x0008
#define ADJ_STATUS 0x0010
#define ADJ_TIMECONST 0x0020
#define ADJ_TAI 0x0080
#define ADJ_SETOFFSET 0x0100
#define ADJ_MICRO 0x1000
#define ADJ_NANO 0x2000
#define ADJ_TICK 0x4000
#define ADJ_OFFSET_SINGLESHOT 0x8001
#define ADJ_OFFSET_SS_READ 0xa001
/* linux/timex.h: clock states (return values) */
#define TIME_OK 0
#define TIME_ERROR 5
/* linux/timex.h: MAXFREQ_SCALED = 500 ppm << 16 */
#define MAXFREQ_SCALED (500L << 16)

/* struct ustat, linux/types.h (x86-64: 32 bytes) */
struct k_ustat {
    i64 f_tfree;
    u64 f_tinode;
    char f_fname[6];
    char f_fpack[6];
    int pad;
};

/* struct statfs, asm-generic/statfs.h (x86-64: 120 bytes) */
struct k_statfs {
    i64 f_type, f_bsize;
    u64 f_blocks, f_bfree, f_bavail, f_files, f_ffree;
    int f_fsid[2];
    i64 f_namelen, f_frsize, f_flags, f_spare[4];
};

/* struct acct_v3, linux/acct.h (64 bytes) */
struct k_acct_v3 {
    unsigned char ac_flag;
    unsigned char ac_version;
    unsigned short ac_tty;
    unsigned ac_exitcode;
    unsigned ac_uid, ac_gid, ac_pid, ac_ppid;
    unsigned ac_btime;
    unsigned ac_etime; /* a float */
    unsigned short ac_utime, ac_stime, ac_mem, ac_io, ac_rw, ac_minflt, ac_majflt, ac_swaps;
    char ac_comm[16];
};
/* linux/acct.h: ac_flag bits */
#define AFORK 0x01
#define ASU 0x02
#define ACORE 0x08
#define AXSIG 0x10

/* linux/syslog.h action numbers, syslog(2) */
#define SYSLOG_ACTION_CLOSE 0
#define SYSLOG_ACTION_OPEN 1
#define SYSLOG_ACTION_READ 2
#define SYSLOG_ACTION_READ_ALL 3
#define SYSLOG_ACTION_READ_CLEAR 4
#define SYSLOG_ACTION_CLEAR 5
#define SYSLOG_ACTION_CONSOLE_OFF 6
#define SYSLOG_ACTION_CONSOLE_ON 7
#define SYSLOG_ACTION_CONSOLE_LEVEL 8
#define SYSLOG_ACTION_SIZE_UNREAD 9
#define SYSLOG_ACTION_SIZE_BUFFER 10

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
/* A child that runs while the parent waits (a fork whose parent sleeps until it exits). */
static __attribute__((unused)) i64 fork_sync(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    return syscall5(SYS_clone, CLONE_VFORK | SIGCHLD, 0, 0, 0, 0);
}
/* Become an unprivileged user (uid/gid 65534) if root; as any other user nothing
 * changes. Either way the caller has no capabilities afterwards. */
static __attribute__((unused)) void become_user(void) {
    if (syscall0(SYS_geteuid) == 0) {
        syscall3(SYS_setresgid, 65534, 65534, 65534);
        syscall3(SYS_setresuid, 65534, 65534, 65534);
    }
}
/* st_dev of a path (struct stat starts with st_dev, asm/stat.h). */
static __attribute__((unused)) u64 dev_of(const char *path) {
    u64 st[18];
    if (syscall4(SYS_newfstatat, AT_FDCWD, path, st, 0) != 0) return 0;
    return st[0];
}
static __attribute__((unused)) i64 iabs(i64 v) { return v < 0 ? -v : v; }
static __attribute__((unused)) i64 realtime_us(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec * 1000000L + t.tv_nsec / 1000;
}

int test_main(void) {
    i64 r;
    struct k_timespec nsec_big = { 1, 1000000000L };
    r = syscall2(SYS_clock_settime, CLOCK_REALTIME, &nsec_big);
    check("nsec_1e9_einval", r == -EINVAL, r);
    struct k_timespec nsec_neg = { 1, -1 };
    r = syscall2(SYS_clock_settime, CLOCK_REALTIME, &nsec_neg);
    check("nsec_negative_einval", r == -EINVAL, r);
    struct k_timespec sec_neg = { -1, 0 };
    r = syscall2(SYS_clock_settime, CLOCK_REALTIME, &sec_neg);
    check("sec_negative_einval", r == -EINVAL, r);
    struct k_timespec huge = { 0x7fffffffffffL, 0 };
    r = syscall2(SYS_clock_settime, CLOCK_REALTIME, &huge);
    check("sec_huge_einval", r == -EINVAL, r);
    r = syscall2(SYS_clock_settime, CLOCK_REALTIME, (void *)8);
    check("bad_ptr_efault", r == -EFAULT, r);

    i64 *res = shared_page(4096);
    i64 pid = fork_sync();
    if (pid == 0) {
        become_user();
        if (syscall0(SYS_geteuid) == 0) exit_group(3);
        struct k_timespec ts;
        syscall2(SYS_clock_gettime, CLOCK_REALTIME, &ts);
        res[0] = syscall2(SYS_clock_settime, CLOCK_REALTIME, &ts);
        res[1] = syscall2(SYS_clock_settime, CLOCK_REALTIME, (void *)8);
        res[2] = syscall2(SYS_clock_settime, CLOCK_REALTIME, &nsec_big);
        res[3] = syscall2(SYS_clock_settime, CLOCK_MONOTONIC, &ts);
        exit_group(0);
    }
    int code = wait_exit(pid);
    check("child_unprivileged", code == 0, code);
    check("user_set_eperm", res[0] == -EPERM, res[0]);
    check("user_bad_ptr_efault", res[1] == -EFAULT, res[1]);
    check("user_bad_nsec_einval", res[2] == -EINVAL, res[2]);
    check("user_monotonic_einval", res[3] == -EINVAL, res[3]);
    return finish();
}
