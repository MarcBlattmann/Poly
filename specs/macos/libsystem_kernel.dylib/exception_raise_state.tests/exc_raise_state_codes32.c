/* poly-mac: libsystem
 * The messages of a task exception port with EXCEPTION_STATE (id 2402): no port descriptors, the flavor and the thread state in the message; the handler replies with the state it wants the thread to continue with (the pc advanced past the instruction, the test register set) and KERN_SUCCESS: the thread continues with that state. */
#include "poly_mexc.h"
int test_main(void) {
    exc_handler_t h;
    exc_handler_start(&h);
    h.set_reg = 1;
    h.reg_val = 0x1234567890abcdefUL;
    exc_seen_t s[3];
    kern_return_t kr = exc_three_faults(&h, EXCEPTION_STATE, s);
    check("task_set_exception_ports succeeds", kr == KERN_SUCCESS, kr);
    exc_check_layout(2402, 0, s, &h);
    check("each fault was resumed: the thread went on", h.count == 3, h.count);
    check("the register the handler set is what the thread stored", exc_reg_out == 0x1234567890abcdefUL, (long long)exc_reg_out);
    return finish();
}
