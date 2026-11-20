/* Mach exceptions, EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES (ADR
 * 0238): the parent registers a task exception port for EXC_BAD_ACCESS and
 * spawns itself; the child inherits the port and writes to address 0x10.
 * The parent's MIG server (mach_exc_server, generated from the SDK's
 * mach_exc.defs) gets the exception with the codes KERN_INVALID_ADDRESS
 * and 0x10 and the thread's state, reads the state again with
 * thread_get_state, and replies KERN_SUCCESS with rip at recovered(): the
 * child goes on there and exits with 7. */
#include "support.h"
#include "mach_exc.h"
#include <mach/thread_status.h>

static int got_exception;
static long got_code0, got_code1;
static int got_flavor, got_count;
static unsigned long got_rip;
static int get_state_kr = -1;
static unsigned long get_state_rip;

__attribute__((noreturn, noinline)) static void recovered(void) { poly_exit(7); }

kern_return_t catch_mach_exception_raise_state_identity(
    mach_port_t port, mach_port_t thread, mach_port_t task, exception_type_t exception,
    mach_exception_data_t code, mach_msg_type_number_t code_count, int *flavor,
    thread_state_t old_state, mach_msg_type_number_t old_count,
    thread_state_t new_state, mach_msg_type_number_t *new_count) {
    (void)port; (void)task; (void)code_count;
    got_exception = exception;
    got_code0 = code[0];
    got_code1 = code[1];
    got_flavor = *flavor;
    got_count = (int)old_count;
    x86_thread_state64_t *old = (x86_thread_state64_t *)old_state;
    got_rip = old->__rip;
    x86_thread_state64_t again;
    mach_msg_type_number_t n = x86_THREAD_STATE64_COUNT;
    get_state_kr = thread_get_state(thread, x86_THREAD_STATE64, (thread_state_t)&again, &n);
    get_state_rip = again.__rip;
    x86_thread_state64_t *out = (x86_thread_state64_t *)new_state;
    *out = *old;
    out->__rip = (unsigned long)recovered;
    out->__rsp = (old->__rsp & ~15UL) - 8;
    *new_count = x86_THREAD_STATE64_COUNT;
    return KERN_SUCCESS;
}
kern_return_t catch_mach_exception_raise(mach_port_t p, mach_port_t th, mach_port_t t, exception_type_t e,
                                         mach_exception_data_t c, mach_msg_type_number_t n) {
    (void)p; (void)th; (void)t; (void)e; (void)c; (void)n;
    return KERN_FAILURE;
}
kern_return_t catch_mach_exception_raise_state(mach_port_t p, exception_type_t e, const mach_exception_data_t c,
                                               mach_msg_type_number_t n, int *f, const thread_state_t o,
                                               mach_msg_type_number_t on, thread_state_t ns,
                                               mach_msg_type_number_t *nn) {
    (void)p; (void)e; (void)c; (void)n; (void)f; (void)o; (void)on; (void)ns; (void)nn;
    return KERN_FAILURE;
}
extern boolean_t mach_exc_server(mach_msg_header_t *in, mach_msg_header_t *out);

int main(int argc, char **argv) {
    mach_task_self_ = (mach_port_t)poly_sys(MACH(28), 0, 0, 0, 0, 0, 0);
    if (argc > 1) {
        *(volatile int *)0x10 = 1;
        poly_exit(1);
    }
    mach_port_t port = 0;
    check("a receive right", mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &port) == KERN_SUCCESS, 0);
    mach_port_insert_right(mach_task_self(), port, port, MACH_MSG_TYPE_MAKE_SEND);
    kern_return_t kr = task_set_exception_ports(mach_task_self(), EXC_MASK_BAD_ACCESS, port,
                                                EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES, x86_THREAD_STATE64);
    check("task_set_exception_ports: KERN_SUCCESS", kr == KERN_SUCCESS, kr);
    exception_mask_t masks[EXC_TYPES_COUNT];
    mach_port_t ports[EXC_TYPES_COUNT];
    exception_behavior_t behaviors[EXC_TYPES_COUNT];
    thread_state_flavor_t flavors[EXC_TYPES_COUNT];
    mach_msg_type_number_t count = EXC_TYPES_COUNT;
    kr = task_get_exception_ports(mach_task_self(), EXC_MASK_ALL, masks, &count, ports, behaviors, flavors);
    check("task_get_exception_ports: KERN_SUCCESS", kr == KERN_SUCCESS, kr);
    check("one handler", count == 1, count);
    check("for EXC_MASK_BAD_ACCESS", masks[0] == EXC_MASK_BAD_ACCESS, masks[0]);
    check("the port registered", ports[0] == port, ports[0]);
    check("the behaviour", behaviors[0] == (int)(EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES), behaviors[0]);
    check("the flavor", flavors[0] == x86_THREAD_STATE64, flavors[0]);

    char *child_argv[] = {argv[0], "child", 0};
    int pid = spawn(argv[0], child_argv);
    check("the child is spawned", pid > 0, pid);
    union {
        mach_msg_header_t hdr;
        char bytes[4096];
    } req, rep;
    kr = mach_msg(&req.hdr, MACH_RCV_MSG, 0, sizeof req, port, 0, 0);
    check("the exception message arrives", kr == KERN_SUCCESS, kr);
    check("mach_exception_raise_state_identity (2407)", req.hdr.msgh_id == 2407, req.hdr.msgh_id);
    check("the MIG server takes it", mach_exc_server(&req.hdr, &rep.hdr), 0);
    check("EXC_BAD_ACCESS", got_exception == EXC_BAD_ACCESS, got_exception);
    check("code KERN_INVALID_ADDRESS", got_code0 == KERN_INVALID_ADDRESS, got_code0);
    check("subcode: the address", got_code1 == 0x10, got_code1);
    check("the flavor", got_flavor == x86_THREAD_STATE64, got_flavor);
    check("the state's size", got_count == x86_THREAD_STATE64_COUNT, got_count);
    check("thread_get_state of the stopped thread", get_state_kr == KERN_SUCCESS, get_state_kr);
    check("the same rip as the message", get_state_rip == got_rip && got_rip != 0, (long)get_state_rip);
    kr = mach_msg(&rep.hdr, MACH_SEND_MSG, rep.hdr.msgh_size, 0, 0, 0, 0);
    check("the reply is sent", kr == KERN_SUCCESS, kr);
    int st = wait_child(pid);
    check("the child went on at recovered() and exited 7", W_EXITED(st) && W_EXITSTATUS(st) == 7, st);
    return finish();
}
