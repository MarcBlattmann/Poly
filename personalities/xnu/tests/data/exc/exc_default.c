/* Mach exceptions, EXCEPTION_DEFAULT with 32-bit codes (ADR 0238): the
 * parent registers a task exception port for EXC_BREAKPOINT and
 * EXC_BAD_INSTRUCTION and spawns itself. The child runs int3: the parent's
 * MIG server (exc_server, from the SDK's exc.defs) gets exception_raise
 * with EXC_BREAKPOINT / EXC_I386_BPT, the thread and the task, and replies
 * KERN_SUCCESS: the child goes on after the int3. Then the child runs
 * ud2: EXC_BAD_INSTRUCTION / EXC_I386_INVOP, which the parent declines
 * (KERN_FAILURE): the child gets the default action and does not exit. */
#include "support.h"
#include "exc.h"

static int calls, got_exception[2], got_code[2], identity_ok[2];

kern_return_t catch_exception_raise(mach_port_t port, mach_port_t thread, mach_port_t task, exception_type_t exception,
                                    exception_data_t code, mach_msg_type_number_t code_count) {
    (void)port; (void)code_count;
    int i = calls < 2 ? calls : 1;
    got_exception[i] = exception;
    got_code[i] = code[0];
    identity_ok[i] = thread != MACH_PORT_NULL && task != MACH_PORT_NULL && thread != task;
    calls++;
    return exception == EXC_BREAKPOINT ? KERN_SUCCESS : KERN_FAILURE;
}
kern_return_t catch_exception_raise_state(mach_port_t p, exception_type_t e, const exception_data_t c,
                                          mach_msg_type_number_t n, int *f, const thread_state_t o,
                                          mach_msg_type_number_t on, thread_state_t ns, mach_msg_type_number_t *nn) {
    (void)p; (void)e; (void)c; (void)n; (void)f; (void)o; (void)on; (void)ns; (void)nn;
    return KERN_FAILURE;
}
kern_return_t catch_exception_raise_state_identity(mach_port_t p, mach_port_t th, mach_port_t t, exception_type_t e,
                                                   exception_data_t c, mach_msg_type_number_t n, int *f,
                                                   thread_state_t o, mach_msg_type_number_t on, thread_state_t ns,
                                                   mach_msg_type_number_t *nn) {
    (void)p; (void)th; (void)t; (void)e; (void)c; (void)n; (void)f; (void)o; (void)on; (void)ns; (void)nn;
    return KERN_FAILURE;
}
extern boolean_t exc_server(mach_msg_header_t *in, mach_msg_header_t *out);

static void serve_one(mach_port_t port, const char *what) {
    union {
        mach_msg_header_t hdr;
        char bytes[4096];
    } req, rep;
    kern_return_t kr = mach_msg(&req.hdr, MACH_RCV_MSG, 0, sizeof req, port, 0, 0);
    check(what, kr == KERN_SUCCESS && req.hdr.msgh_id == 2401, kr ? kr : req.hdr.msgh_id);
    exc_server(&req.hdr, &rep.hdr);
    mach_msg(&rep.hdr, MACH_SEND_MSG, rep.hdr.msgh_size, 0, 0, 0, 0);
}

int main(int argc, char **argv) {
    mach_task_self_ = (mach_port_t)poly_sys(MACH(28), 0, 0, 0, 0, 0, 0);
    if (argc > 1) {
        __asm__ volatile("int3");
        __asm__ volatile("ud2");
        poly_exit(1);
    }
    mach_port_t port = 0;
    mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &port);
    mach_port_insert_right(mach_task_self(), port, port, MACH_MSG_TYPE_MAKE_SEND);
    kern_return_t kr = task_set_exception_ports(mach_task_self(), EXC_MASK_BREAKPOINT | EXC_MASK_BAD_INSTRUCTION, port,
                                                EXCEPTION_DEFAULT, THREAD_STATE_NONE);
    check("task_set_exception_ports: KERN_SUCCESS", kr == KERN_SUCCESS, kr);
    char *child_argv[] = {argv[0], "child", 0};
    int pid = spawn(argv[0], child_argv);
    check("the child is spawned", pid > 0, pid);
    serve_one(port, "exception_raise for the int3 (2401)");
    check("EXC_BREAKPOINT", got_exception[0] == EXC_BREAKPOINT, got_exception[0]);
    check("EXC_I386_BPT", got_code[0] == EXC_I386_BPT, got_code[0]);
    check("with the thread and the task", identity_ok[0], 0);
    serve_one(port, "exception_raise for the ud2 (2401)");
    check("EXC_BAD_INSTRUCTION", got_exception[1] == EXC_BAD_INSTRUCTION, got_exception[1]);
    check("EXC_I386_INVOP", got_code[1] == EXC_I386_INVOP, got_code[1]);
    int st = wait_child(pid);
    check("declined: the child did not exit on its own", !(W_EXITED(st) && W_EXITSTATUS(st) <= 1), st);
    value("the child's wait status", st);
    return finish();
}
