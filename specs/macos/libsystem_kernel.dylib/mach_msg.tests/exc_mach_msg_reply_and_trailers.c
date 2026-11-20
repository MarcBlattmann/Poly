/* poly-mac: libsystem
 * A reply port sent as MAKE_SEND_ONCE arrives as the remote port with a MOVE_SEND_ONCE disposition (18) in the remote slot of the bits (the local slot says
 * the send disposition of the port received on); a send and a receive in one call (MACH_SEND_MSG | MACH_RCV_MSG), as a client does, sends the request and
 * returns the reply; the trailer elements asked for (MACH_RCV_TRAILER_ELEMENTS) give trailers of 8 (null), 12 (sequence number), 20 (sender), 52 (audit),
 * 60 (context) and 68 (attribute) bytes, in format 0; the sequence number of the trailer counts the messages received. */
#include "poly_mexc.h"
typedef struct {
    mach_msg_header_t h;
    char pay[16];
} msg_t;
typedef struct {
    mach_msg_header_t h;
    char pay[16];
    unsigned char tr[200];
} rcv_t;
static kern_return_t send_msg(mach_port_t dst, int id, mach_port_t reply, int reply_disp) {
    msg_t m;
    memset(&m, 0, sizeof m);
    m.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, reply_disp);
    m.h.msgh_remote_port = dst;
    m.h.msgh_local_port = reply;
    m.h.msgh_id = id;
    m.h.msgh_size = sizeof m;
    return mach_msg(&m.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof m, 0, MACH_PORT_NULL, 100, MACH_PORT_NULL);
}
static void *server(void *arg) {
    mach_port_t port = (mach_port_t)(long)arg;
    rcv_t r;
    if (mach_msg(&r.h, MACH_RCV_MSG, 0, sizeof r, port, MACH_MSG_TIMEOUT_NONE, MACH_PORT_NULL)) return NULL;
    msg_t reply;
    memset(&reply, 0, sizeof reply);
    reply.h.msgh_bits = MACH_MSGH_BITS(MACH_MSGH_BITS_REMOTE(r.h.msgh_bits), 0);
    reply.h.msgh_remote_port = r.h.msgh_remote_port;
    reply.h.msgh_id = r.h.msgh_id + 100;
    reply.h.msgh_size = sizeof reply.h;
    mach_msg(&reply.h, MACH_SEND_MSG, sizeof reply.h, 0, MACH_PORT_NULL, MACH_MSG_TIMEOUT_NONE, MACH_PORT_NULL);
    return NULL;
}
int test_main(void) {
    mach_port_t p = exc_new_port(), reply = exc_new_port();
    rcv_t r;
    send_msg(p, 11, reply, MACH_MSG_TYPE_MAKE_SEND_ONCE);
    memset(&r, 0, sizeof r);
    kern_return_t kr = mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, p, 100, MACH_PORT_NULL);
    check("a message with a reply port arrives", kr == KERN_SUCCESS, kr);
    check("the reply port arrives as a new name in the remote port field", r.h.msgh_remote_port != MACH_PORT_NULL && r.h.msgh_remote_port != reply, r.h.msgh_remote_port);
    check("with a send-once disposition (18) in the remote slot and the send disposition in the local slot", MACH_MSGH_BITS_REMOTE(r.h.msgh_bits) == MACH_MSG_TYPE_PORT_SEND_ONCE && MACH_MSGH_BITS_LOCAL(r.h.msgh_bits) == MACH_MSG_TYPE_PORT_SEND, r.h.msgh_bits);
    check("the name is a send-once right here", exc_type_of(r.h.msgh_remote_port) == MACH_PORT_TYPE_SEND_ONCE, exc_type_of(r.h.msgh_remote_port));
    const int elements[] = {MACH_RCV_TRAILER_NULL, MACH_RCV_TRAILER_SEQNO, MACH_RCV_TRAILER_SENDER, MACH_RCV_TRAILER_AUDIT, MACH_RCV_TRAILER_CTX, MACH_RCV_TRAILER_AV};
    const unsigned sizes[] = {8, 12, 20, 52, 60, 68};
    for (int i = 0; i < 6; i++) {
        send_msg(p, 20 + i, MACH_PORT_NULL, 0);
        memset(&r, 0, sizeof r);
        kr = mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT | MACH_RCV_TRAILER_TYPE(MACH_MSG_TRAILER_FORMAT_0) | MACH_RCV_TRAILER_ELEMENTS(elements[i]), 0, sizeof r, p, 100, MACH_PORT_NULL);
        mach_msg_trailer_t *tr = (mach_msg_trailer_t *)((char *)&r.h + r.h.msgh_size);
        char name[80];
        snprintf(name, sizeof name, "trailer elements %d: a trailer of %u bytes", elements[i], sizes[i]);
        check(name, kr == KERN_SUCCESS && tr->msgh_trailer_size == sizes[i] && tr->msgh_trailer_type == 0, kr ? kr : (long long)tr->msgh_trailer_size);
    }
    mach_port_t srv = exc_new_port();
    pthread_t t;
    pthread_create(&t, NULL, server, (void *)(long)srv);
    msg_t req;
    memset(&req, 0, sizeof req);
    mach_port_t client = exc_new_port();
    req.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, MACH_MSG_TYPE_MAKE_SEND_ONCE);
    req.h.msgh_remote_port = srv;
    req.h.msgh_local_port = client;
    req.h.msgh_id = 40;
    req.h.msgh_size = sizeof req.h;
    kr = mach_msg(&req.h, MACH_SEND_MSG | MACH_RCV_MSG, sizeof req.h, sizeof r, client, MACH_MSG_TIMEOUT_NONE, MACH_PORT_NULL);
    check("a request and its reply in one call: the reply comes back in the same buffer", kr == KERN_SUCCESS && req.h.msgh_id == 140, req.h.msgh_id);
    pthread_join(t, NULL);
    return finish();
}
