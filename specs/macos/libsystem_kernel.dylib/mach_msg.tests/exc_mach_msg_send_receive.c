/* poly-mac: libsystem
 * mach_msg to and from a port the program made: a message of a header and 16 bytes sent with MACH_SEND_MSG arrives with its id, its size (40) and
 * its bytes, the local port set to the receive right's name and the remote port MACH_PORT_NULL, the bits holding the dispositions as the
 * receiver sees them; the default trailer is 8 bytes; messages come in the order sent; a port set receives for its members and says which member
 * in the local port. */
#include "poly_mexc.h"
typedef struct {
    mach_msg_header_t h;
    char pay[16];
} msg_t;
typedef struct {
    mach_msg_header_t h;
    char pay[16];
    mach_msg_trailer_t tr;
} rcv_t;
static kern_return_t send_msg(mach_port_t dst, int id, mach_port_t reply, int reply_disp) {
    msg_t m;
    memset(&m, 0, sizeof m);
    m.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, reply_disp);
    m.h.msgh_remote_port = dst;
    m.h.msgh_local_port = reply;
    m.h.msgh_id = id;
    m.h.msgh_size = sizeof m;
    memcpy(m.pay, "0123456789abcdef", 16);
    return mach_msg(&m.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof m, 0, MACH_PORT_NULL, 100, MACH_PORT_NULL);
}
int test_main(void) {
    mach_port_t p = exc_new_port();
    check("a send succeeds", send_msg(p, 5, MACH_PORT_NULL, 0) == KERN_SUCCESS, 0);
    rcv_t r;
    memset(&r, 0xee, sizeof r);
    kern_return_t kr = mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, p, 100, MACH_PORT_NULL);
    check("the receive succeeds", kr == KERN_SUCCESS, kr);
    check("id 5, size 40", r.h.msgh_id == 5 && r.h.msgh_size == sizeof(msg_t), r.h.msgh_size);
    check("the bytes arrived", memcmp(r.pay, "0123456789abcdef", 16) == 0, 0);
    check("remote port MACH_PORT_NULL, local port the receive right's name", r.h.msgh_remote_port == MACH_PORT_NULL && r.h.msgh_local_port == p, r.h.msgh_local_port);
    check("the bits: a send disposition in the local slot, nothing in the remote slot", MACH_MSGH_BITS_LOCAL(r.h.msgh_bits) == MACH_MSG_TYPE_PORT_SEND && MACH_MSGH_BITS_REMOTE(r.h.msgh_bits) == 0, r.h.msgh_bits);
    check("the default trailer is 8 bytes of format 0", r.tr.msgh_trailer_type == 0 && r.tr.msgh_trailer_size == 8, r.tr.msgh_trailer_size);
    for (int i = 1; i <= 3; i++) send_msg(p, 100 + i, MACH_PORT_NULL, 0);
    int ok = 1;
    for (int i = 1; i <= 3; i++) ok &= mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, p, 100, MACH_PORT_NULL) == KERN_SUCCESS && r.h.msgh_id == 100 + i;
    check("three messages come in the order sent", ok, 0);
    mach_port_t set = MACH_PORT_NULL, member = exc_new_port();
    mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_PORT_SET, &set);
    mach_port_insert_member(mach_task_self(), member, set);
    send_msg(member, 77, MACH_PORT_NULL, 0);
    kr = mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, set, 100, MACH_PORT_NULL);
    check("a receive on a port set gets a member's message, naming the member", kr == KERN_SUCCESS && r.h.msgh_id == 77 && r.h.msgh_local_port == member, kr);
    return finish();
}
