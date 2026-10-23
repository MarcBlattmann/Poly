/* A thread of the process (clone with CLONE_THREAD) gets EPERM from
 * delete_module too, and so does the main thread while the other thread
 * is alive and after it has finished.
 * poly-diff: syscalls=off (two threads)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_delete_module 176
/* linux/capability.h */
#define CAP_VERSION_3 0x20080522
#define CAP_SYS_MODULE 16

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static __attribute__((unused)) i64 delete_module(const char *name, u64 flags) {
    return syscall2(SYS_delete_module, name, flags);
}

/* Empty all three capability sets (any user may do that; root then fails
 * every privilege check too) and confirm with capget that nothing is
 * left: 1 when the drop worked. */
static __attribute__((unused)) int drop_caps(void) {
    struct cap_header h = { CAP_VERSION_3, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return 0;
    h.version = CAP_VERSION_3;
    h.pid = 0;
    memset(d, 0xff, sizeof d);
    if (syscall2(SYS_capget, &h, d) != 0) return 0;
    return d[0].effective == 0 && d[1].effective == 0 && d[0].permitted == 0 && d[1].permitted == 0;
}

/* Set the effective and permitted sets of capabilities 0-31 (32-63 and
 * the inheritable set become empty); the capset result. */
static __attribute__((unused)) int set_caps(unsigned eff, unsigned prm) {
    struct cap_header h = { CAP_VERSION_3, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    d[0].effective = eff;
    d[0].permitted = prm;
    return (int)syscall2(SYS_capset, &h, d);
}

static __attribute__((unused)) void *map_pages(u64 n, int prot) {
    i64 r = syscall6(SYS_mmap, 0, n * 4096, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    return r < 0 ? (void *)0 : (void *)r;
}

/* Names no module on any system has. */
#define NO_SUCH_MODULE "poly_no_such_module_q7"
static volatile i64 thread_result = 1;
static volatile unsigned thread_done_gate;
static char thread_stack[16384] __attribute__((aligned(16)));

static int thread_fn(void *arg) {
    (void)arg;
    thread_result = delete_module(NO_SUCH_MODULE, O_NONBLOCK);
    gate_open(&thread_done_gate);
    return 0;
}

int test_main(void) {
    int dropped = drop_caps();
    check("caps_dropped", dropped, dropped);
    if (!dropped) return finish();

    int ptid = 0;
    static int ctid;
    ctid = 0;
    i64 tid = poly_clone(THREAD_FLAGS, thread_stack + sizeof thread_stack, &ptid, &ctid, thread_fn, 0);
    check("clone", tid > 0, tid);
    if (tid <= 0) return finish();
    i64 r = delete_module(0, 0);
    check("main_while_thread_eperm", r == -EPERM, r);
    gate_wait(&thread_done_gate);
    check("thread_eperm", thread_result == -EPERM, thread_result);
    r = delete_module(NO_SUCH_MODULE, O_NONBLOCK | O_TRUNC);
    check("main_after_eperm", r == -EPERM, r);
    return finish();
}
