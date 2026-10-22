/* Time fields of a record: ac_btime is the process start in seconds since
 * the Epoch (CLOCK_REALTIME, within a couple of seconds of fork);
 * ac_etime is the elapsed time as an IEEE float in units of 1/100 s: a
 * child that sleeps 300 ms shows at least 20 and well under 1000. The
 * comp_t CPU times of a sleeping child are small (under 100 = 1 s). Run
 * as pid 1 of a new pid namespace.
 * poly-diff: needs=root (only root may switch process accounting)
 */
#include "poly_ipc.h"

#define SYS_acct 163
#define SYS_newfstatat 262
#define SYS_renameat 264
#define CLONE_NEWNS 0x00020000
#define CLONE_NEWPID 0x20000000
#define PR_SET_NAME 15
#define AFORK 0x01 /* linux/acct.h */
#define AXSIG 0x10 /* linux/acct.h */
#define REC 64     /* sizeof(struct acct_v3) */

/* struct acct_v3, linux/acct.h and acct(5) */
struct acct_v3 {
    unsigned char flag;
    unsigned char version;
    unsigned short tty;
    unsigned exitcode, uid, gid, pid, ppid, btime;
    float etime;
    unsigned short utime, stime, mem, io, rw, minflt, majflt, swaps;
    char comm[16];
};

struct k_stat {
    u64 dev, ino, nlink;
    unsigned mode, uid, gid, pad0;
    u64 rdev;
    i64 size, blksize, blocks;
    u64 times[6];
    i64 unused[3];
};

static __attribute__((unused)) i64 file_size(const char *path) {
    struct k_stat st;
    i64 r = syscall4(SYS_newfstatat, AT_FDCWD, path, &st, 0);
    return r < 0 ? r : st.size;
}

static __attribute__((unused)) void make_file(const char *path, unsigned mode) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, mode));
}

/* Reads up to `max` records from offset `off` of `path`; returns the count. */
static __attribute__((unused)) int read_records(const char *path, i64 off, struct acct_v3 *out, int max) {
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_RDONLY, 0);
    if (fd < 0) return -1;
    syscall3(SYS_lseek, fd, off, 0);
    i64 n = read((int)fd, out, (u64)max * REC);
    close((int)fd);
    return n < 0 ? -1 : (int)(n / REC);
}

/* A child that names itself, leaves its session (no controlling tty) and
 * exits with `code`. Returns its pid after it has been reaped. */
static __attribute__((unused)) i64 run_child(const char *name, int code) {
    i64 pid = fork();
    if (pid == 0) {
        syscall0(SYS_setsid);
        syscall2(SYS_prctl, PR_SET_NAME, name);
        exit_group(code);
    }
    wait_status(pid);
    return pid;
}

/* Run `body` as pid 1 of a new pid namespace (plus `extra` clone flags),
 * so the accounting it switches on covers only its own namespace. The
 * child never calls body unless it really is pid 1 of a fresh namespace.
 * Returns the child's exit code. */
static __attribute__((unused)) int in_pid_ns(u64 extra, void (*body)(void)) {
    i64 pid = syscall5(SYS_clone, CLONE_NEWPID | extra | SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        i64 me = getpid();
        check("pid_1_in_new_ns", me == 1, me);
        if (me != 1) exit_group(1);
        body();
        exit_group(failed ? 1 : 0);
    }
    check("clone_newpid", pid > 0, pid);
    if (pid < 0) return -1;
    return wait_exit(pid);
}
#define F "/tmp/acct_times"

static i64 wall_s(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}

static u64 comp(unsigned short c) { return (u64)(c & 0x1fff) << (((c >> 13) & 7) * 3); }

static void body(void) {
    i64 r = syscall1(SYS_acct, F);
    check("on_ok", r == 0, r);
    i64 t0 = wall_s();
    i64 pid = fork();
    if (pid == 0) {
        syscall2(SYS_prctl, PR_SET_NAME, "sleeper");
        sleep_ms(300);
        exit_group(0);
    }
    wait_status(pid);
    i64 t1 = wall_s();
    r = syscall1(SYS_acct, 0);
    check("off_ok", r == 0, r);
    struct acct_v3 rec[2];
    int n = read_records(F, 0, rec, 2);
    check("read_records", n >= 1 && str_eq(rec[0].comm, "sleeper"), n);
    if (n < 1) return;
    check("btime_window", (i64)rec[0].btime >= t0 - 2 && (i64)rec[0].btime <= t1 + 2, rec[0].btime);
    i64 et = (i64)rec[0].etime;
    check("etime_at_least_20", rec[0].etime >= 20.0f, et);
    check("etime_below_1000", rec[0].etime < 1000.0f, et);
    check("utime_small", comp(rec[0].utime) < 100, rec[0].utime);
    check("stime_small", comp(rec[0].stime) < 100, rec[0].stime);
}

int test_main(void) {
    make_file(F, 0644);
    int code = in_pid_ns(0, body);
    check("child", code == 0, code);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
