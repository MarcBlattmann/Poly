/* A MACH_RECV source on a receive right fires when a message is queued;
 * the message is received with mach_msg in the handler. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <mach/mach.h>

typedef struct {
    mach_msg_header_t head;
    int payload;
} send_msg_t;
typedef struct {
    mach_msg_header_t head;
    int payload;
    mach_msg_trailer_t trailer;
} recv_msg_t;

int test_main(void) {
    mach_port_t port = MACH_PORT_NULL;
    check("allocate a receive right", mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &port) == KERN_SUCCESS, 0);
    check("add a send right", mach_port_insert_right(mach_task_self(), port, port, MACH_MSG_TYPE_MAKE_SEND) == KERN_SUCCESS, 0);
    dispatch_queue_t q = dispatch_queue_create("com.poly.mach", DISPATCH_QUEUE_SERIAL);
    dispatch_source_t s = dispatch_source_create(DISPATCH_SOURCE_TYPE_MACH_RECV, port, 0, q);
    check("a mach receive source is made", s != NULL, 0);
    dispatch_semaphore_t got = dispatch_semaphore_create(0);
    __block int payload = -1, kr = -1, id = -1;
    dispatch_source_set_event_handler(s, ^{
        recv_msg_t m;
        memset(&m, 0, sizeof m);
        kr = mach_msg(&m.head, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof m, port, 0, MACH_PORT_NULL);
        payload = m.payload;
        id = m.head.msgh_id;
        dispatch_semaphore_signal(got);
    });
    dispatch_resume(s);
    check("no message yet: the source did not fire", dispatch_semaphore_wait(got, dispatch_time(DISPATCH_TIME_NOW, 150 * NSEC_PER_MSEC)) != 0, 0);
    send_msg_t m;
    memset(&m, 0, sizeof m);
    m.head.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    m.head.msgh_size = sizeof m;
    m.head.msgh_remote_port = port;
    m.head.msgh_id = 1234;
    m.payload = 99;
    check("send a message to the port", mach_msg(&m.head, MACH_SEND_MSG, sizeof m, 0, MACH_PORT_NULL, MACH_MSG_TIMEOUT_NONE, MACH_PORT_NULL) == KERN_SUCCESS, 0);
    check("the source fired", gcd_wait(got), 0);
    check("the handler received the message with mach_msg", kr == MACH_MSG_SUCCESS, kr);
    check("its payload (99) and id (1234)", payload == 99 && id == 1234, payload);
    dispatch_source_cancel(s);
    return finish();
}
