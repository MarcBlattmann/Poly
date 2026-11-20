/* poly-mac: libsystem
 * The receive right bootstrap_check_in returns is an ordinary one with a queue limit of 2: messages sent to the looked-up send right arrive on it in order, a third finds the queue full. */
#include "poly_mapi.h"
int test_main(void) {
    char name[100];
    service_name(name, sizeof name, "ci_recv");
    mach_port_t recv = 0, send = 0;
    (void)!bootstrap_check_in(bootstrap_port, name, &recv);
    (void)!bootstrap_look_up(bootstrap_port, name, &send);
    check("two messages can be sent", send_empty(send, 1, 1000) == 0 && send_empty(send, 2, 1000) == 0, 0);
    check("a third finds the queue full (limit 2): MACH_SEND_TIMED_OUT", send_empty(send, 3, 100) == MACH_SEND_TIMED_OUT, 0);
    poly_rcv_t m;
    int ok = 1;
    for (int i = 1; i <= 2; i++) ok &= recv_msg(recv, 1000, &m) == KERN_SUCCESS && m.h.msgh_id == i;
    check("the two arrive in order", ok, 0);
    check("then the queue is empty", recv_msg(recv, 100, &m) == MACH_RCV_TIMED_OUT, 0);
    return finish();
}
