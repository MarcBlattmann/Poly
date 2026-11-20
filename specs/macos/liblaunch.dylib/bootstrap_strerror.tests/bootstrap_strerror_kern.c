/* poly-mac: libsystem
 * bootstrap_strerror of a Mach kern_return_t or message error gives mach_error_string's text for it ("(os/kern) failure",
 * "(ipc/send) invalid destination port", ...). */
#include "poly_mapi.h"
static int is(int code, const char *text) {
    const char *s = bootstrap_strerror(code);
    return s && strcmp(s, text) == 0;
}
int test_main(void) {
    check("1 is (os/kern) invalid address", is(1, "(os/kern) invalid address"), 0);
    check("5 is (os/kern) failure", is(5, "(os/kern) failure"), 0);
    check("15 is (os/kern) invalid name", is(15, "(os/kern) invalid name"), 0);
    check("17 is (os/kern) invalid right", is(17, "(os/kern) invalid right"), 0);
    check("18 is (os/kern) invalid value", is(18, "(os/kern) invalid value"), 0);
    check("MACH_SEND_INVALID_DEST is (ipc/send) invalid destination port", is(MACH_SEND_INVALID_DEST, "(ipc/send) invalid destination port"), 0);
    check("MACH_RCV_TIMED_OUT is (ipc/rcv) timed out", is(MACH_RCV_TIMED_OUT, "(ipc/rcv) timed out"), 0);
    check("it is the same text mach_error_string gives", strcmp(bootstrap_strerror(15), mach_error_string(15)) == 0, 0);
    return finish();
}
