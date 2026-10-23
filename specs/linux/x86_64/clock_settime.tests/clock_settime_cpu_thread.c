/* CPU-time clocks of a second thread of the caller's process: its thread
 * clock ((~tid << 3) | 4..6) exists and is EPERM; the same tid used as a
 * process clock ((~tid << 3) | 0..2) names no process and is EINVAL.
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126

/* First tv_sec that CLOCK_REALTIME refuses: 2^63 ns in seconds, less 30
 * years of 365 days (observed boundary on x86-64). */
#define CS_SEC_LIMIT 8277292036LL

static A_UNUSED i64 cs(i64 clock, const void *tp) { return syscall2(SYS_clock_settime, clock, tp); }

/* CPU-time clock id of a process (type 0..2) or thread (type 4..6). */
static A_UNUSED i64 cpu_clock(i64 pid, int type) { return (i64)(i32)(u32)((~(u64)pid << 3) | (u64)type); }

/* Clock id of an open file descriptor ("dynamic" clock). */
static A_UNUSED i64 fd_clock(i64 fd) { return (i64)(i32)(u32)((~(u64)fd << 3) | 3); }

static A_UNUSED void fill(void *p, u8 v, size_t n) {
    u8 *b = p;
    for (size_t i = 0; i < n; i++) b[i] = v;
}

/* Empty all three capability sets (any user may) and confirm the
 * effective set is empty: root then fails the privilege check too, and
 * can never change the clock by accident. Returns 0 when that worked. */
static A_UNUSED int drop_caps(void) {
    struct { u32 version; i32 pid; } h = { 0x20080522, 0 };
    struct { u32 effective, permitted, inheritable; } d[2];
    fill(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return -1;
    fill(d, 0xff, sizeof d);
    if (syscall2(SYS_capget, &h, d) != 0) return -1;
    return (d[0].effective | d[1].effective) == 0 ? 0 : -1;
}

/* Two pages, the second inaccessible: returns the start of the first. */
static A_UNUSED u8 *edge_pages(void) {
    u8 *p = (u8 *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall3(SYS_mprotect, (i64)(p + A_PAGE), A_PAGE, PROT_NONE);
    return p;
}

/* CLOCK_REALTIME minus CLOCK_MONOTONIC, in microseconds. */
static A_UNUSED i64 real_minus_mono_us(void) {
    struct a_timespec r, m;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    return (r.tv_sec - m.tv_sec) * 1000000 + (r.tv_nsec - m.tv_nsec) / 1000;
}

static volatile i32 stop, tid_word;

static int thread(void *arg) {
    (void)arg;
    while (!stop) a_relax();
    return 0;
}

int test_main(void) {
    if (drop_caps() != 0) {
        check("drop_caps", 0, -1);
        return finish();
    }
    struct a_timespec good = {5, 0};
    tid_word = -1;
    i64 t = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                        CLONE_CHILD_CLEARTID,
                    a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&tid_word, 0, thread, 0);
    check("thread_started", t > 0, t);
    if (t <= 0) return finish();
    for (int ty = 4; ty <= 6; ty++) expect("thread_clock_eperm", cs(cpu_clock(t, ty), &good), -EPERM);
    for (int ty = 0; ty <= 2; ty++) expect("tid_as_process_clock_einval", cs(cpu_clock(t, ty), &good), -EINVAL);
    i64 pid = syscall0(SYS_getpid);
    expect("leader_thread_clock_eperm", cs(cpu_clock(pid, 6), &good), -EPERM);
    expect("process_clock_eperm", cs(cpu_clock(pid, 2), &good), -EPERM);
    stop = 1;
    while (tid_word) a_relax();
    return finish();
}
