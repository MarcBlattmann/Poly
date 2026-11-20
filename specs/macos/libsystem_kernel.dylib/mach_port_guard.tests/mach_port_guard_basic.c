/* poly-mac: libsystem
 * mach_port_guard(task, name, guard, strict): guards a receive right whose context is 0; the guard becomes the context
 * (mach_port_get_context returns it, 0 when strict); guarding again is KERN_INVALID_ARGUMENT; mach_port_unguard with the guard
 * undoes it (the context is 0 again) and the port can be guarded anew. Guard 0 is accepted and guards nothing. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_receive();
    mach_port_context_t ctx = 9;
    kern_return_t kr = mach_port_guard(t, p, 0x1000, 0);
    check("guard succeeds", kr == KERN_SUCCESS, kr);
    mach_port_get_context(t, p, &ctx);
    check("the guard is the context", ctx == 0x1000, (long)ctx);
    kr = mach_port_guard(t, p, 0x2000, 0);
    check("guarding a guarded port: KERN_INVALID_ARGUMENT (4)", kr == KERN_INVALID_ARGUMENT, kr);
    mach_port_get_context(t, p, &ctx);
    check("and the guard is unchanged", ctx == 0x1000, (long)ctx);
    kr = mach_port_unguard(t, p, 0x1000);
    check("unguard with the guard succeeds", kr == KERN_SUCCESS, kr);
    mach_port_get_context(t, p, &ctx);
    check("the context is 0 again", ctx == 0, (long)ctx);
    check("guard anew works", mach_port_guard(t, p, 0x3000, 0) == KERN_SUCCESS, 0);
    mach_port_get_context(t, p, &ctx);
    check("with the new guard", ctx == 0x3000, (long)ctx);
    mach_port_unguard(t, p, 0x3000);
    check("strict guard succeeds", mach_port_guard(t, p, 0x4000, 1) == KERN_SUCCESS, 0);
    ctx = 7;
    mach_port_get_context(t, p, &ctx);
    check("a strict guard reads as context 0", ctx == 0, (long)ctx);
    check("and unguards with its value", mach_port_unguard(t, p, 0x4000) == KERN_SUCCESS, 0);
    mach_port_t q = new_receive();
    mach_port_set_context(t, q, 0x55);
    kr = mach_port_guard(t, q, 0x1000, 0);
    check("a port whose context is not 0 cannot be guarded: KERN_INVALID_ARGUMENT", kr == KERN_INVALID_ARGUMENT, kr);
    mach_port_t r = new_receive();
    check("guard value 0 is accepted", mach_port_guard(t, r, 0, 0) == KERN_SUCCESS, 0);
    check("and destruct with guard 0 works", mach_port_destruct(t, r, 0, 0) == KERN_SUCCESS, 0);
    return finish();
}
