/* poly-mac: libsystem
 * Receiving: a buffer too small for the message and the trailer is MACH_RCV_TOO_LARGE (0x10004004); without MACH_RCV_LARGE the message is destroyed and
 * the header filled in (its size and id), with MACH_RCV_LARGE the message stays queued and the header says its size so that it can be received again
 * with a bigger buffer; an empty queue with MACH_RCV_TIMEOUT gives MACH_RCV_TIMED_OUT (0x10004003), also with a timeout of 0; a receive on a name that
 * is not a receive right (nothing, MACH_PORT_NULL, a send right only) is MACH_RCV_INVALID_NAME (0x10004002); a receive right destroyed during a wait
 * ends the wait with MACH_RCV_PORT_CHANGED (0x10004006; not MACH_RCV_PORT_DIED), by mach_port_destroy as well as by mod_refs or destruct; when the
 * destroyed port is a member of the port set being waited on, the wait goes on (to its timeout). */
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
static void send_msg(mach_port_t dst, int id) {
    msg_t m;
    memset(&m, 0, sizeof m);
    m.h.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    m.h.msgh_remote_port = dst;
    m.h.msgh_id = id;
    m.h.msgh_size = sizeof m;
    mach_msg(&m.h, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof m, 0, MACH_PORT_NULL, 100, MACH_PORT_NULL);
}
static mach_port_t task_self_send_only(void) { return mach_task_self(); }
static void *destroyer(void *arg) {
    usleep(200000);
    mach_port_destroy(mach_task_self(), (mach_port_t)(long)arg);
    return NULL;
}
int test_main(void) {
    mach_port_t p = exc_new_port();
    mach_msg_header_t small;
    send_msg(p, 8);
    memset(&small, 0, sizeof small);
    kern_return_t kr = mach_msg(&small, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof small, p, 100, MACH_PORT_NULL);
    check("a buffer of a header only: MACH_RCV_TOO_LARGE", kr == MACH_RCV_TOO_LARGE, kr);
    check("the header has the message's id", small.msgh_id == 8, small.msgh_id);
    value("the size the header holds after that", small.msgh_size);
    rcv_t r;
    check("the message is gone: a timeout follows", mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, p, 50, MACH_PORT_NULL) == MACH_RCV_TIMED_OUT, 0);
    send_msg(p, 9);
    memset(&small, 0, sizeof small);
    kr = mach_msg(&small, MACH_RCV_MSG | MACH_RCV_LARGE | MACH_RCV_TIMEOUT, 0, sizeof small, p, 100, MACH_PORT_NULL);
    check("with MACH_RCV_LARGE too: MACH_RCV_TOO_LARGE", kr == MACH_RCV_TOO_LARGE, kr);
    check("the header says the size the buffer has to have (message and trailer 8)", small.msgh_size == sizeof(msg_t), small.msgh_size);
    kr = mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_LARGE | MACH_RCV_TIMEOUT, 0, sizeof r, p, 100, MACH_PORT_NULL);
    check("the message was kept and a bigger buffer gets it", kr == KERN_SUCCESS && r.h.msgh_id == 9, kr);
    check("an empty queue with a timeout of 0: MACH_RCV_TIMED_OUT", mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, p, 0, MACH_PORT_NULL) == MACH_RCV_TIMED_OUT, 0);
    check("with a timeout of 50 ms: MACH_RCV_TIMED_OUT", mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, p, 50, MACH_PORT_NULL) == MACH_RCV_TIMED_OUT, 0);
    check("a name that is nothing: MACH_RCV_INVALID_NAME", mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, 0x4321, 10, MACH_PORT_NULL) == MACH_RCV_INVALID_NAME, 0);
    check("MACH_PORT_NULL: MACH_RCV_INVALID_NAME", mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, MACH_PORT_NULL, 10, MACH_PORT_NULL) == MACH_RCV_INVALID_NAME, 0);
    check("a send right only: MACH_RCV_INVALID_NAME", mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, task_self_send_only(), 10, MACH_PORT_NULL) == MACH_RCV_INVALID_NAME, 0);
    mach_port_t q = exc_new_port();
    pthread_t t;
    pthread_create(&t, NULL, destroyer, (void *)(long)q);
    kr = mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, q, 3000, MACH_PORT_NULL);
    pthread_join(t, NULL);
    check("the port destroyed during the wait: MACH_RCV_PORT_CHANGED", kr == MACH_RCV_PORT_CHANGED, kr);
    mach_port_t set = MACH_PORT_NULL, member = exc_new_port();
    mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_PORT_SET, &set);
    mach_port_insert_member(mach_task_self(), member, set);
    pthread_create(&t, NULL, destroyer, (void *)(long)member);
    kr = mach_msg(&r.h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof r, set, 800, MACH_PORT_NULL);
    pthread_join(t, NULL);
    check("a member of the port set destroyed during the wait: the wait goes on to its timeout", kr == MACH_RCV_TIMED_OUT, kr);
    return finish();
}
