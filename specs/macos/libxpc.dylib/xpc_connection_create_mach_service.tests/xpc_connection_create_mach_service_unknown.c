/* poly-mac: libsystem
 * xpc_connection_create_mach_service of a name that no job declares, as a client (flags 0): it returns a connection object (type
 * "connection"); once resumed its event handler gets one error, XPC_ERROR_CONNECTION_INVALID ("Connection invalid"); a message sent
 * with xpc_connection_send_message_with_reply_sync gets that same error object back, every time. */
#include "poly_mapi.h"
int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("poly.xcm1", NULL);
    char name[100];
    service_name(name, sizeof name, "xcm_unknown");
    xpc_connection_t c = xpc_connection_create_mach_service(name, q, 0);
    check("a connection object", c != NULL && xpc_get_type(c) == XPC_TYPE_CONNECTION, 0);
    check("its type is named connection", strcmp(xpc_type_get_name(xpc_get_type(c)), "connection") == 0, 0);
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block xpc_object_t seen = NULL;
    xpc_connection_set_event_handler(c, ^(xpc_object_t ev) {
        if (!seen) seen = ev;
        dispatch_semaphore_signal(sem);
    });
    xpc_connection_resume(c);
    check("the handler is called", xpc_wait(sem, 5), 0);
    check("with an error object", seen != NULL && xpc_get_type(seen) == XPC_TYPE_ERROR, 0);
    check("which is XPC_ERROR_CONNECTION_INVALID", seen == XPC_ERROR_CONNECTION_INVALID, 0);
    check("described as Connection invalid", strcmp(xpc_error_text(seen), "Connection invalid") == 0, 0);
    xpc_object_t m = xpc_dictionary_create_empty();
    xpc_dictionary_set_int64(m, "n", 1);
    xpc_object_t r1 = xpc_connection_send_message_with_reply_sync(c, m), r2 = xpc_connection_send_message_with_reply_sync(c, m);
    check("a sync send gets the same error object", r1 == XPC_ERROR_CONNECTION_INVALID && r2 == XPC_ERROR_CONNECTION_INVALID, 0);
    xpc_release(m);
    return finish();
}
