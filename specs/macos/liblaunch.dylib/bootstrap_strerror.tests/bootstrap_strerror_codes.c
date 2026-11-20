/* poly-mac: libsystem
 * bootstrap_strerror of the bootstrap server's own codes (<servers/bootstrap.h>): 0 is "Success", 1100 to 1105 have a text each,
 * the numbers around them are "unknown error code". */
#include "poly_mapi.h"
static int is(int code, const char *text) {
    const char *s = bootstrap_strerror(code);
    return s && strcmp(s, text) == 0;
}
int test_main(void) {
    check("0 is Success", is(0, "Success"), 0);
    check("1100 is Permission denied (BOOTSTRAP_NOT_PRIVILEGED)", is(1100, "Permission denied"), 0);
    check("1101 is Service name already exists", is(1101, "Service name already exists"), 0);
    check("1102 is Unknown service name", is(1102, "Unknown service name"), 0);
    check("1103 is Service is already active", is(1103, "Service is already active"), 0);
    check("1104 is Too many lookups were requested in one request", is(1104, "Too many lookups were requested in one request"), 0);
    check("1105 is Out of memory", is(1105, "Out of memory"), 0);
    check("1099 is unknown error code", is(1099, "unknown error code"), 0);
    check("1106 is unknown error code", is(1106, "unknown error code"), 0);
    check("2000 is unknown error code", is(2000, "unknown error code"), 0);
    return finish();
}
