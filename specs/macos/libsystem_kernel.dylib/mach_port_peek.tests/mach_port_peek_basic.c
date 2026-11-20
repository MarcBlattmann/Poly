/* poly-mac: libsystem
 * mach_port_peek(task, receive name, MACH_RCV_TRAILER_NULL, &seqno, &size, &id, &trailer, &count) looks at a queued message
 * without taking it: with seqno 0 the first one (its seqno comes back), with a seqno the message with that number; the size is the
 * message and its 8-byte trailer; nothing is removed. A seqno that is not queued, or an empty queue, is KERN_FAILURE (5). */
#include "poly_mapi.h"
static kern_return_t peek(mach_port_t p, mach_port_seqno_t *sq, mach_msg_size_t *sz, mach_msg_id_t *id) {
    mach_msg_trailer_t tr;
    mach_msg_type_number_t c = sizeof tr;
    return mach_port_peek(mach_task_self(), p, MACH_RCV_TRAILER_NULL, sq, sz, id, (mach_msg_trailer_info_t)&tr, &c);
}
int test_main(void) {
    mach_port_t p = new_port_with_send();
    mach_port_seqno_t sq = 0;
    mach_msg_size_t sz = 0;
    mach_msg_id_t id = 0;
    check("an empty queue: KERN_FAILURE", peek(p, &sq, &sz, &id) == KERN_FAILURE, 0);
    send_empty(p, 11, 100);
    send_empty(p, 22, 100);
    send_empty(p, 33, 100);
    kern_return_t kr = peek(p, &sq, &sz, &id);
    check("seqno 0 peeks the first", kr == KERN_SUCCESS && id == 11 && sq == 0, id);
    check("size is 24 + 8 (header and trailer)", sz == 32, sz);
    sq = 0;
    peek(p, &sq, &sz, &id);
    check("peeking again gives the same message", id == 11, id);
    sq = 2;
    kr = peek(p, &sq, &sz, &id);
    check("seqno 2 peeks the third", kr == KERN_SUCCESS && id == 33 && sq == 2, id);
    sq = 3;
    kr = peek(p, &sq, &sz, &id);
    check("seqno 3 is not queued: KERN_FAILURE", kr == KERN_FAILURE, kr);
    poly_rcv_t m;
    recv_msg(p, 500, &m);
    check("receive gets the first, nothing was removed by peeking", m.h.msgh_id == 11, m.h.msgh_id);
    sq = 0;
    kr = peek(p, &sq, &sz, &id);
    check("after it, seqno 0 peeks the next and says its number", kr == KERN_SUCCESS && id == 22 && sq == 1, sq);
    mach_port_t q = new_port_with_send();
    struct { mach_msg_header_t h; char pay[40]; } big;
    memset(&big, 0, sizeof big);
    big.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    big.h.msgh_size = sizeof big;
    big.h.msgh_remote_port = q;
    big.h.msgh_id = 77;
    mach_msg(&big.h, MACH_SEND_MSG, sizeof big, 0, MACH_PORT_NULL, 0, MACH_PORT_NULL);
    sq = 0;
    peek(q, &sq, &sz, &id);
    check("a 64-byte message: size 72", sz == 72 && id == 77, sz);
    return finish();
}
