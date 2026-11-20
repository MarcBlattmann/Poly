/* poly-mac: libsystem
 * The sync argument of MACH_NOTIFY_NO_SENDERS: it only matters when the request is made. With no senders and a make-send count
 * below sync nothing comes at once (the request is kept and fires when a send right made later is gone); with send rights
 * outstanding it does not matter, the message comes when they are gone. A second request replaces the first and returns the
 * old one's notify right as a send-once name; asking with MACH_PORT_NULL cancels, again returning the old right. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self(), prev = 0;
    poly_rcv_t m;
    mach_port_t a = new_receive(), nt = new_receive();
    mach_port_insert_right(t, a, a, MACH_MSG_TYPE_MAKE_SEND);
    mach_port_mod_refs(t, a, MACH_PORT_RIGHT_SEND, -1);
    mach_port_request_notification(t, a, MACH_NOTIFY_NO_SENDERS, 5, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("make-send count 1 below sync 5, no senders: nothing at once", recv_msg(nt, 100, &m) == MACH_RCV_TIMED_OUT, 0);
    mach_port_insert_right(t, a, a, MACH_MSG_TYPE_MAKE_SEND);
    mach_port_mod_refs(t, a, MACH_PORT_RIGHT_SEND, -1);
    check("a send right made and dropped: the message comes", recv_msg(nt, 500, &m) == KERN_SUCCESS && m.h.msgh_id == 70, 0);
    mach_port_t b = new_port_with_send(), nt2 = new_receive();
    mach_port_request_notification(t, b, MACH_NOTIFY_NO_SENDERS, 3, nt2, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    mach_port_mod_refs(t, b, MACH_PORT_RIGHT_SEND, -1);
    check("a send right outstanding at the request: the sync value does not hold the message back", recv_msg(nt2, 500, &m) == KERN_SUCCESS && m.h.msgh_id == 70, 0);
    mach_port_t c = new_port_with_send(), n1 = new_receive(), n2 = new_receive();
    kern_return_t kr = mach_port_request_notification(t, c, MACH_NOTIFY_NO_SENDERS, 0, n1, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("first request: previous is MACH_PORT_NULL", kr == KERN_SUCCESS && prev == MACH_PORT_NULL, prev);
    kr = mach_port_request_notification(t, c, MACH_NOTIFY_NO_SENDERS, 0, n2, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("second request: previous is the first one's send-once right", kr == KERN_SUCCESS && type_of(prev) == MACH_PORT_TYPE_SEND_ONCE, type_of(prev));
    mach_port_t p2 = prev;
    kr = mach_port_request_notification(t, c, MACH_NOTIFY_NO_SENDERS, 0, MACH_PORT_NULL, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("cancelling with MACH_PORT_NULL returns the second one's right", kr == KERN_SUCCESS && type_of(prev) == MACH_PORT_TYPE_SEND_ONCE && prev != p2, kr);
    mach_port_mod_refs(t, c, MACH_PORT_RIGHT_SEND, -1);
    check("nothing arrives on the first notify port", recv_msg(n1, 100, &m) == MACH_RCV_TIMED_OUT, 0);
    check("nothing arrives on the second either", recv_msg(n2, 100, &m) == MACH_RCV_TIMED_OUT, 0);
    return finish();
}
