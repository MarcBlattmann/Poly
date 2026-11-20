/* poly-mac: libsystem
 * Calling exception_raise (the client stub of the exception message with EXCEPTION_DEFAULT) from user space on a port the
 * program made itself, with ports it made itself for the thread and the task, does not return: the process is ended with
 * SIGKILL (a Mach port guard violation reported as INVALID_OPTIONS). With MACH_PORT_NULL as the port it returns
 * MACH_SEND_INVALID_DEST (0x10000003). */
#include "poly_mexc.h"
#include <mach/exc.h>
int test_main(void) {
    if (is_child("child")) {
        exc_no_crash_report();
        mach_port_t port = exc_new_port(), a = exc_new_port(), b = exc_new_port();
        exception_data_type_t codes[2] = {1, 2};
        if (exc_child_arg()[0] == 'n') {
            kern_return_t kr = exception_raise(MACH_PORT_NULL, a, b, EXC_SOFTWARE, codes, 2);
            return kr == MACH_SEND_INVALID_DEST ? 0 : 3;
        }
        exception_raise(port, a, b, EXC_SOFTWARE, codes, 2);
        return 5; /* it came back */
    }
    check("with MACH_PORT_NULL as the port: the call returns MACH_SEND_INVALID_DEST", exc_wait(exc_start_self("child", "null")) == 0, 0);
    int st = exc_wait(exc_start_self("child", "plain"));
    check("with a port of the program's own: the process is killed by SIGKILL", st == 1000 + SIGKILL, st);
    return finish();
}
