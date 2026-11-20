/* poly-mac: libsystem
 * Cancelling the client connection gives both ends an event: the client's handler and the server's per-peer handler each get
 * XPC_ERROR_CONNECTION_INVALID (the server's after the client's cancel reached it). Events arrive on the connection's queue, not on
 * the caller's thread. */
#include "poly_mapi.h"
int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("poly.xsend3", NULL);
    dispatch_semaphore_t server_gone = dispatch_semaphore_create(0), client_gone = dispatch_semaphore_create(0), got_msg = dispatch_semaphore_create(0);
    xpc_connection_t l = xpc_connection_create(NULL, q);
    xpc_connection_set_event_handler(l, ^(xpc_object_t ev) {
        if (xpc_get_type(ev) != XPC_TYPE_CONNECTION) return;
        xpc_connection_t peer = (xpc_connection_t)ev;
        xpc_connection_set_event_handler(peer, ^(xpc_object_t msg) {
            if (xpc_get_type(msg) == XPC_TYPE_DICTIONARY) {
                dispatch_semaphore_signal(got_msg);
                xpc_object_t reply = xpc_dictionary_create_reply(msg);
                xpc_connection_send_message(xpc_dictionary_get_remote_connection(msg), reply);
                xpc_release(reply);
            } else if (msg == XPC_ERROR_CONNECTION_INVALID) {
                dispatch_semaphore_signal(server_gone);
            }
        });
        xpc_connection_resume(peer);
    });
    xpc_connection_resume(l);
    xpc_connection_t c = xpc_connection_create_from_endpoint(xpc_endpoint_create(l));
    xpc_connection_set_event_handler(c, ^(xpc_object_t ev) {
        if (ev == XPC_ERROR_CONNECTION_INVALID) dispatch_semaphore_signal(client_gone);
    });
    xpc_connection_resume(c);
    xpc_object_t m = xpc_dictionary_create_empty();
    xpc_object_t r = xpc_connection_send_message_with_reply_sync(c, m);
    check("the server saw the message", xpc_wait(got_msg, 5), 0);
    check("and answered", xpc_get_type(r) == XPC_TYPE_DICTIONARY, 0);
    xpc_connection_cancel(c);
    check("the client's handler gets XPC_ERROR_CONNECTION_INVALID", xpc_wait(client_gone, 5), 0);
    check("so does the server's handler for that connection", xpc_wait(server_gone, 5), 0);
    xpc_release(r);
    xpc_release(m);
    return finish();
}
