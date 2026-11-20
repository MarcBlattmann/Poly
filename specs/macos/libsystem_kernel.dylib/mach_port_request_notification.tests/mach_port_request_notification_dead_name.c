/* poly-mac: libsystem
 * MACH_NOTIFY_DEAD_NAME: asked of a name that holds a send right; when the port's receive right goes a message with id
 * MACH_NOTIFY_DEAD_NAME (72, 36 bytes: header, NDR record, the name) arrives on the notify port and the name is a dead name
 * (type 0x100000, two user references when the task also had the receive right). A second request replaces the first and returns
 * its send-once right. On a name that is already dead the call fails with KERN_INVALID_ARGUMENT and the send-once right given
 * is used up: a MACH_NOTIFY_SEND_ONCE message (71, header only) arrives instead. */
#include "poly_mapi.h"
static unsigned name_in(poly_rcv_t *m) {
    unsigned v;
    memcpy(&v, m->body + 8, 4);
    return v;
}
int test_main(void) {
    mach_port_t t = mach_task_self(), prev = 0x7777;
    poly_rcv_t m;
    mach_port_t a = new_port_with_send(), nt = new_receive();
    kern_return_t kr = mach_port_request_notification(t, a, MACH_NOTIFY_DEAD_NAME, 0, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("the request succeeds, no previous", kr == KERN_SUCCESS && prev == MACH_PORT_NULL, kr);
    check("nothing while the port lives", recv_msg(nt, 100, &m) == MACH_RCV_TIMED_OUT, 0);
    mach_port_mod_refs(t, a, MACH_PORT_RIGHT_RECEIVE, -1);
    check("the name is a dead name", type_of(a) == MACH_PORT_TYPE_DEAD_NAME, type_of(a));
    check("with two user references", urefs_of(a, MACH_PORT_RIGHT_DEAD_NAME) == 2, urefs_of(a, MACH_PORT_RIGHT_DEAD_NAME));
    check("the notification comes", recv_msg(nt, 500, &m) == KERN_SUCCESS, 0);
    check("id 72, 36 bytes", m.h.msgh_id == MACH_NOTIFY_DEAD_NAME && m.h.msgh_size == 36, m.h.msgh_id);
    check("it names the dead name", name_in(&m) == a, name_in(&m));
    mach_port_t b = new_port_with_send(), n1 = new_receive(), n2 = new_receive();
    mach_port_request_notification(t, b, MACH_NOTIFY_DEAD_NAME, 0, n1, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    kr = mach_port_request_notification(t, b, MACH_NOTIFY_DEAD_NAME, 0, n2, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("a second request returns the first's send-once right", kr == KERN_SUCCESS && type_of(prev) == MACH_PORT_TYPE_SEND_ONCE, type_of(prev));
    mach_port_mod_refs(t, b, MACH_PORT_RIGHT_RECEIVE, -1);
    check("only the second notify port hears of it", recv_msg(n1, 100, &m) == MACH_RCV_TIMED_OUT && recv_msg(n2, 500, &m) == KERN_SUCCESS && m.h.msgh_id == 72, 0);
    mach_port_t c = new_port_with_send(), n3 = new_receive();
    mach_port_mod_refs(t, c, MACH_PORT_RIGHT_RECEIVE, -1);
    kr = mach_port_request_notification(t, c, MACH_NOTIFY_DEAD_NAME, 0, n3, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("on a name that is already dead: KERN_INVALID_ARGUMENT (4)", kr == KERN_INVALID_ARGUMENT, kr);
    check("the right given is used up: a send-once notification (71), header only", recv_msg(n3, 500, &m) == KERN_SUCCESS && m.h.msgh_id == MACH_NOTIFY_SEND_ONCE && m.h.msgh_size == sizeof(mach_msg_header_t), m.h.msgh_id);
    return finish();
}
