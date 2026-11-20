/* task_for_pid, Mach trap 45: asked for the caller's own pid it succeeds and returns the caller's own task port name (the one task_self_trap gives), a send right. */
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
    u32 kr = task_for_pid(self(), mypid(), &t);
    check("task_for_pid of the caller's own pid succeeds", kr == KERN_SUCCESS, kr);
    check("the port is the name task_self_trap gives", t == (u32)self(), t);
    check("a send right", type_of(t) == MACH_PORT_TYPE_SEND, type_of(t));
    u32 t2 = 0;
    check("asked again, the same name", task_for_pid(self(), mypid(), &t2) == KERN_SUCCESS && t2 == t, t2);
    return finish();
}
