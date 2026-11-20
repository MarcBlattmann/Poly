/* poly-mac: libsystem
 * The options of mach_port_construct: MPO_INSERT_SEND_RIGHT makes a send right too (one user reference),
 * MPO_QLIMIT sets the queue limit (0 to 1024 are accepted, 1025 is KERN_INVALID_VALUE and creates nothing),
 * MPO_CONTEXT_AS_GUARD makes the context argument the guard (mach_port_get_context returns it), flags that mean nothing are ignored. */
#include "poly_mapi.h"
static unsigned qlimit_of(mach_port_t p) {
    mach_port_limits_t l;
    mach_msg_type_number_t c = MACH_PORT_LIMITS_INFO_COUNT;
    mach_port_get_attributes(mach_task_self(), p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, &c);
    return l.mpl_qlimit;
}
int test_main(void) {
    mach_port_t t = mach_task_self(), p = 0;
    mach_port_options_t o;
    memset(&o, 0, sizeof o);
    o.flags = MPO_INSERT_SEND_RIGHT;
    check("INSERT_SEND_RIGHT succeeds", mach_port_construct(t, &o, 0, &p) == KERN_SUCCESS, 0);
    check("receive and send", type_of(p) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND), type_of(p));
    check("one user reference", urefs_of(p, MACH_PORT_RIGHT_SEND) == 1, urefs_of(p, MACH_PORT_RIGHT_SEND));
    mach_port_destruct(t, p, -1, 0);
    check("destruct with send_right_delta -1 takes both", is_gone(p), type_of(p));
    memset(&o, 0, sizeof o);
    o.flags = MPO_QLIMIT;
    o.mpl.mpl_qlimit = 3;
    mach_port_construct(t, &o, 0, &p);
    check("QLIMIT 3", qlimit_of(p) == 3, qlimit_of(p));
    mach_port_destruct(t, p, 0, 0);
    o.mpl.mpl_qlimit = MACH_PORT_QLIMIT_MAX;
    check("QLIMIT 1024 is accepted", mach_port_construct(t, &o, 0, &p) == KERN_SUCCESS && qlimit_of(p) == 1024, qlimit_of(p));
    mach_port_destruct(t, p, 0, 0);
    o.mpl.mpl_qlimit = 0;
    kern_return_t kr = mach_port_construct(t, &o, 0, &p);
    check("QLIMIT 0 is accepted", kr == KERN_SUCCESS, kr);
    value("queue limit when 0 was asked", qlimit_of(p));
    mach_port_destruct(t, p, 0, 0);
    p = 0x7777;
    o.mpl.mpl_qlimit = MACH_PORT_QLIMIT_MAX + 1;
    kr = mach_port_construct(t, &o, 0, &p);
    check("QLIMIT 1025 is KERN_INVALID_VALUE (18)", kr == KERN_INVALID_VALUE, kr);
    check("and the out name is left alone", p == 0x7777, p);
    memset(&o, 0, sizeof o);
    o.flags = MPO_CONTEXT_AS_GUARD;
    check("CONTEXT_AS_GUARD succeeds", mach_port_construct(t, &o, 0x7a7a, &p) == KERN_SUCCESS, 0);
    mach_port_context_t ctx = 0;
    mach_port_get_context(t, p, &ctx);
    check("the guard is the port's context", ctx == 0x7a7a, (long)ctx);
    check("destruct with the guard works", mach_port_destruct(t, p, 0, 0x7a7a) == KERN_SUCCESS && is_gone(p), 0);
    memset(&o, 0, sizeof o);
    o.flags = 0x8000;
    kr = mach_port_construct(t, &o, 0, &p);
    check("a flag bit that means nothing is ignored", kr == KERN_SUCCESS && type_of(p) == MACH_PORT_TYPE_RECEIVE, kr);
    mach_port_destruct(t, p, 0, 0);
    memset(&o, 0, sizeof o);
    o.flags = MPO_INSERT_SEND_RIGHT | MPO_QLIMIT | MPO_CONTEXT_AS_GUARD;
    o.mpl.mpl_qlimit = 2;
    check("all three together", mach_port_construct(t, &o, 0x55, &p) == KERN_SUCCESS && type_of(p) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND) && qlimit_of(p) == 2, 0);
    mach_port_destruct(t, p, -1, 0x55);
    check("and destruct with delta -1 and the guard takes it away", is_gone(p), type_of(p));
    return finish();
}
