/* poly-mac: libsystem
 * mach_port_get_attributes(task, receive name, MACH_PORT_RECEIVE_STATUS, &status, &count) with count 10: a fresh port has
 * everything 0 and queue limit 5; mps_mscount counts MAKE_SEND (not COPY_SEND), mps_msgcount the queued messages, mps_seqno the
 * messages received so far, mps_sorights the send-once rights made, mps_pset 1 while the port is in a port set;
 * mach_port_set_mscount sets the make-send count. */
#include "poly_mapi.h"
static mach_port_status_t status(mach_port_t p, kern_return_t *kr, mach_msg_type_number_t *count) {
    mach_port_status_t s;
    memset(&s, 0xee, sizeof s);
    mach_msg_type_number_t c = MACH_PORT_RECEIVE_STATUS_COUNT;
    *kr = mach_port_get_attributes(mach_task_self(), p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    *count = c;
    return s;
}
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_receive();
    kern_return_t kr;
    mach_msg_type_number_t c;
    mach_port_status_t s = status(p, &kr, &c);
    check("succeeds, count 10", kr == KERN_SUCCESS && c == 10, c);
    check("a fresh port: all zero, queue limit 5", s.mps_pset == 0 && s.mps_seqno == 0 && s.mps_mscount == 0 && s.mps_qlimit == 5 && s.mps_msgcount == 0 && s.mps_sorights == 0 && s.mps_flags == 0, s.mps_qlimit);
    check("no notification requests: nsrequest 0, pdrequest 0", s.mps_nsrequest == 0 && s.mps_pdrequest == 0, s.mps_nsrequest);
    mach_port_insert_right(t, p, p, MACH_MSG_TYPE_MAKE_SEND);
    mach_port_insert_right(t, p, p, MACH_MSG_TYPE_MAKE_SEND);
    mach_port_t ex;
    mach_msg_type_name_t acq;
    mach_port_extract_right(t, p, MACH_MSG_TYPE_COPY_SEND, &ex, &acq);
    s = status(p, &kr, &c);
    check("two MAKE_SENDs and a COPY_SEND: mscount 2", s.mps_mscount == 2, s.mps_mscount);
    send_empty(p, 1, 100);
    send_empty(p, 2, 100);
    send_empty(p, 3, 100);
    s = status(p, &kr, &c);
    check("three queued: msgcount 3, seqno 0", s.mps_msgcount == 3 && s.mps_seqno == 0, s.mps_msgcount);
    poly_rcv_t m;
    recv_msg(p, 500, &m);
    s = status(p, &kr, &c);
    check("one received: msgcount 2, seqno 1", s.mps_msgcount == 2 && s.mps_seqno == 1, s.mps_seqno);
    mach_port_extract_right(t, p, MACH_MSG_TYPE_MAKE_SEND_ONCE, &ex, &acq);
    s = status(p, &kr, &c);
    check("a send-once right made: sorights 1", s.mps_sorights == 1, s.mps_sorights);
    mach_port_t ps = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    mach_port_insert_member(t, p, ps);
    s = status(p, &kr, &c);
    check("in a port set: pset 1", s.mps_pset == 1, s.mps_pset);
    mach_port_set_mscount(t, p, 10);
    s = status(p, &kr, &c);
    check("set_mscount 10: mscount 10", s.mps_mscount == 10, s.mps_mscount);
    return finish();
}
