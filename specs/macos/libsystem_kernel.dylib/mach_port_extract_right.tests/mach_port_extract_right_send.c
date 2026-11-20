/* poly-mac: libsystem
 * mach_port_extract_right(task, name, MAKE_SEND / COPY_SEND, &right, &acquired) of a receive+send name: the right is the same
 * name, acquired is MACH_MSG_TYPE_PORT_SEND (17); MAKE_SEND adds a user reference and a make-send count, COPY_SEND a
 * user reference only. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_receive();
    mach_port_t ex = 0;
    mach_msg_type_name_t acq = 0;
    kern_return_t kr = mach_port_extract_right(t, p, MACH_MSG_TYPE_MAKE_SEND, &ex, &acq);
    check("MAKE_SEND succeeds", kr == KERN_SUCCESS, kr);
    check("the right found is the name itself", ex == p, ex);
    check("acquired as MACH_MSG_TYPE_PORT_SEND", acq == MACH_MSG_TYPE_PORT_SEND, acq);
    check("receive+send, one user reference", type_of(p) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND) && urefs_of(p, MACH_PORT_RIGHT_SEND) == 1, urefs_of(p, MACH_PORT_RIGHT_SEND));
    mach_port_status_t s;
    mach_msg_type_number_t c = MACH_PORT_RECEIVE_STATUS_COUNT;
    mach_port_get_attributes(t, p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("make-send count 1", s.mps_mscount == 1, s.mps_mscount);
    acq = 0;
    kr = mach_port_extract_right(t, p, MACH_MSG_TYPE_COPY_SEND, &ex, &acq);
    check("COPY_SEND succeeds with the same name and PORT_SEND", kr == KERN_SUCCESS && ex == p && acq == MACH_MSG_TYPE_PORT_SEND, kr);
    check("two user references", urefs_of(p, MACH_PORT_RIGHT_SEND) == 2, urefs_of(p, MACH_PORT_RIGHT_SEND));
    c = MACH_PORT_RECEIVE_STATUS_COUNT;
    mach_port_get_attributes(t, p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("make-send count still 1", s.mps_mscount == 1, s.mps_mscount);
    kr = mach_port_extract_right(t, p, MACH_MSG_TYPE_MOVE_SEND, &ex, &acq);
    check("MOVE_SEND succeeds, same name, PORT_SEND", kr == KERN_SUCCESS && ex == p && acq == MACH_MSG_TYPE_PORT_SEND, kr);
    check("the send right is still two references", urefs_of(p, MACH_PORT_RIGHT_SEND) == 2, urefs_of(p, MACH_PORT_RIGHT_SEND));
    kr = mach_port_extract_right(t, bootstrap_port, MACH_MSG_TYPE_COPY_SEND, &ex, &acq);
    check("COPY_SEND of a send-only name works", kr == KERN_SUCCESS && ex == bootstrap_port, kr);
    mach_port_deallocate(t, ex);
    kr = mach_port_extract_right(t, bootstrap_port, MACH_MSG_TYPE_MAKE_SEND, &ex, &acq);
    check("MAKE_SEND of a send-only name: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    return finish();
}
