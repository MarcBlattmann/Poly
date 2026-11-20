/* task_name_for_pid, Mach trap 44, of other processes: the caller's parent and a child that runs work (a send right); pid 1, pid 0 and pids that do not exist are KERN_FAILURE (5); a child that has exited is KERN_FAILURE. */
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
    u32 n = 0x7777;
    i64 ppid = bsd0(SYS_getppid).ret;
    u32 kr = task_name_for_pid(self(), ppid, &n);
    check("the parent works", kr == KERN_SUCCESS, kr);
    check("a send right", type_of(n) == MACH_PORT_TYPE_SEND, type_of(n));
    i64 pid = start_sleeper();
    u32 m = 0;
    kr = task_name_for_pid(self(), pid, &m);
    check("a running child works", kr == KERN_SUCCESS && type_of(m) == MACH_PORT_TYPE_SEND, kr);
    i32 got = 0;
    check("pid_for_task of it gives its pid", pid_for_task(m, &got) == KERN_SUCCESS && got == pid, got);
    stop_sleeper(pid);
    u32 z = 0x7777;
    check("pid 1: KERN_FAILURE", task_name_for_pid(self(), 1, &z) == KERN_FAILURE, 0);
    check("pid 0: KERN_FAILURE", task_name_for_pid(self(), 0, &z) == KERN_FAILURE, 0);
    check("a pid that does not exist: KERN_FAILURE", task_name_for_pid(self(), 99999999, &z) == KERN_FAILURE, 0);
    check("the child that ended: KERN_FAILURE", task_name_for_pid(self(), pid, &z) == KERN_FAILURE, 0);
    check("the out name is set to MACH_PORT_NULL", z == 0, z);
    return finish();
}
