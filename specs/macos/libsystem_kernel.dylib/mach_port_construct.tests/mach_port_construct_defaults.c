/* poly-mac: libsystem
 * mach_port_construct with empty options: a new receive right (nothing else), queue limit 5, context 0, an empty
 * receive status; every call gives a different name; mach_port_destruct(.., 0, 0) takes it away again. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_options_t o;
    memset(&o, 0, sizeof o);
    mach_port_t a = 0, b = 0;
    kern_return_t kr = mach_port_construct(mach_task_self(), &o, 0, &a);
    check("construct succeeds", kr == KERN_SUCCESS, kr);
    check("a receive right and nothing else", type_of(a) == MACH_PORT_TYPE_RECEIVE, type_of(a));
    mach_port_construct(mach_task_self(), &o, 0, &b);
    check("a second call gives another name", a != b && a != 0 && b != 0, a);
    mach_port_limits_t l;
    mach_msg_type_number_t c = MACH_PORT_LIMITS_INFO_COUNT;
    mach_port_get_attributes(mach_task_self(), a, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, &c);
    check("queue limit 5", l.mpl_qlimit == MACH_PORT_QLIMIT_DEFAULT && l.mpl_qlimit == 5, l.mpl_qlimit);
    mach_port_context_t ctx = 99;
    mach_port_get_context(mach_task_self(), a, &ctx);
    check("context 0", ctx == 0, (long)ctx);
    mach_port_status_t s;
    c = MACH_PORT_RECEIVE_STATUS_COUNT;
    mach_port_get_attributes(mach_task_self(), a, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("empty status: no messages, no send rights made, no notification requests",
          s.mps_msgcount == 0 && s.mps_mscount == 0 && s.mps_seqno == 0 && s.mps_sorights == 0 && s.mps_nsrequest == 0 && s.mps_pdrequest == 0 && s.mps_pset == 0, s.mps_msgcount);
    check("destruct takes it away", mach_port_destruct(mach_task_self(), a, 0, 0) == KERN_SUCCESS && is_gone(a), 0);
    check("and the other is untouched", type_of(b) == MACH_PORT_TYPE_RECEIVE, type_of(b));
    mach_port_destruct(mach_task_self(), b, 0, 0);
    return finish();
}
