/* poly-mac: libsystem
 * What mach_port_peek refuses: names not in the task KERN_INVALID_NAME (15); names without a receive right (send only, port set)
 * KERN_INVALID_RIGHT (17); an invalid task MACH_SEND_INVALID_DEST; a trailer count of 0 KERN_INVALID_VALUE (18). */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_seqno_t sq = 0;
    mach_msg_size_t sz = 0;
    mach_msg_id_t id = 0;
    mach_msg_trailer_t tr;
    mach_msg_type_number_t c = sizeof tr;
    kern_return_t kr = mach_port_peek(t, 0x4321, MACH_RCV_TRAILER_NULL, &sq, &sz, &id, (mach_msg_trailer_info_t)&tr, &c);
    check("a name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    c = sizeof tr;
    kr = mach_port_peek(t, bootstrap_port, MACH_RCV_TRAILER_NULL, &sq, &sz, &id, (mach_msg_trailer_info_t)&tr, &c);
    check("a send right only: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t ps = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    c = sizeof tr;
    kr = mach_port_peek(t, ps, MACH_RCV_TRAILER_NULL, &sq, &sz, &id, (mach_msg_trailer_info_t)&tr, &c);
    check("a port set: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t p = new_port_with_send();
    send_empty(p, 5, 100);
    c = 0;
    kr = mach_port_peek(t, p, MACH_RCV_TRAILER_NULL, &sq, &sz, &id, (mach_msg_trailer_info_t)&tr, &c);
    check("a trailer count of 0: KERN_INVALID_VALUE (18)", kr == KERN_INVALID_VALUE, kr);
    c = sizeof tr;
    kr = mach_port_peek(MACH_PORT_NULL, p, MACH_RCV_TRAILER_NULL, &sq, &sz, &id, (mach_msg_trailer_info_t)&tr, &c);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
