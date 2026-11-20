/* task_for_pid, Mach trap 45: for any pid but the caller's own an unprivileged, unentitled caller gets KERN_FAILURE (5) and the out name is set to MACH_PORT_NULL: its parent, pid 1, pid 0, pids that do not exist, negative pids. */
#include "poly_mtest.h"

/* Mach traps (<mach/mach_traps.h>); results are kern_return_t. */
#define MACH_task_name_for_pid 44
#define MACH_task_for_pid 45
#define MACH_pid_for_task 46
#define KERN_FAILURE 5
static __attribute__((unused)) u32 task_for_pid(i64 target, i64 pid, u32 *t) { return (u32)poly_mach6(MACH_task_for_pid, target, pid, (i64)t, 0, 0, 0); }
static __attribute__((unused)) u32 task_name_for_pid(i64 target, i64 pid, u32 *t) { return (u32)poly_mach6(MACH_task_name_for_pid, target, pid, (i64)t, 0, 0, 0); }
static __attribute__((unused)) u32 pid_for_task(i64 task, i32 *pid) { return (u32)poly_mach6(MACH_pid_for_task, task, (i64)pid, 0, 0, 0, 0); }
static __attribute__((unused)) i64 self(void) { return mach0(MACH_task_self_trap); }
static __attribute__((unused)) u32 port_type(i64 task, i64 name, u32 *type) { return (u32)poly_mach6(MACH__kernelrpc_mach_port_type_trap, task, name, (i64)type, 0, 0, 0); }
static __attribute__((unused)) u32 type_of(u32 name) { u32 ty = 0; return port_type(self(), name, &ty) ? 0xdead0000u : ty; }
/* A child that waits until it is killed: this program again with argv[1] = "sleep". */
static __attribute__((unused)) i64 start_sleeper(void) { return poly_start_self("sleep", 0, 0); }
static __attribute__((unused)) void stop_sleeper(i64 pid) { bsd2(SYS_kill, pid, 9); poly_wait(pid); }
static __attribute__((unused)) i64 mypid(void) { return bsd0(SYS_getpid).ret; }

int test_main(void) {
    if (is_child("sleep")) {
        bsd1(SYS_sigsuspend, 0);
        return 0;
    }
    u32 t = 0x7777;
    i64 ppid = bsd0(SYS_getppid).ret;
    check("the parent: KERN_FAILURE", task_for_pid(self(), ppid, &t) == KERN_FAILURE, 0);
    check("pid 1: KERN_FAILURE", task_for_pid(self(), 1, &t) == KERN_FAILURE, 0);
    check("pid 0: KERN_FAILURE", task_for_pid(self(), 0, &t) == KERN_FAILURE, 0);
    check("a pid that does not exist: KERN_FAILURE", task_for_pid(self(), 99999999, &t) == KERN_FAILURE, 0);
    check("pid -1: KERN_FAILURE", task_for_pid(self(), -1, &t) == KERN_FAILURE, 0);
    check("the out name is set to MACH_PORT_NULL", t == 0, t);
    return finish();
}
