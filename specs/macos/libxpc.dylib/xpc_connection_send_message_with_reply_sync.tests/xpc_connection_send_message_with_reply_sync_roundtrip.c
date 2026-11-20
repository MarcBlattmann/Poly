/* poly-mac: libsystem
 * An anonymous listener (xpc_connection_create(NULL, queue)), its endpoint, and a connection made from the endpoint: the listener's
 * handler gets a connection and sets a handler on it; xpc_connection_send_message_with_reply_sync of a dictionary on the client
 * returns the dictionary the handler sent as the reply (made with xpc_dictionary_create_reply); the message sent is not changed;
 * a second message works the same; the server sees each message's values. */
#include "poly_mapi.h"
int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("poly.xsend1", NULL);
    xpc_connection_t l = xpc_connection_create(NULL, q);
    xpc_connection_set_event_handler(l, ^(xpc_object_t ev) {
        if (xpc_get_type(ev) != XPC_TYPE_CONNECTION) return;
        xpc_connection_t peer = (xpc_connection_t)ev;
        xpc_connection_set_event_handler(peer, ^(xpc_object_t m) {
            if (xpc_get_type(m) != XPC_TYPE_DICTIONARY) return;
            xpc_object_t r = xpc_dictionary_create_reply(m);
            xpc_dictionary_set_int64(r, "double", xpc_dictionary_get_int64(m, "n") * 2);
            xpc_dictionary_set_string(r, "echo", xpc_dictionary_get_string(m, "op"));
            xpc_connection_send_message(xpc_dictionary_get_remote_connection(m), r);
            xpc_release(r);
        });
        xpc_connection_resume(peer);
    });
    xpc_connection_resume(l);
    xpc_endpoint_t ep = xpc_endpoint_create(l);
    check("an endpoint", ep != NULL && xpc_get_type(ep) == XPC_TYPE_ENDPOINT, 0);
    xpc_connection_t c = xpc_connection_create_from_endpoint(ep);
    check("a connection from it", c != NULL && xpc_get_type(c) == XPC_TYPE_CONNECTION, 0);
    xpc_connection_set_event_handler(c, ^(xpc_object_t ev) { (void)ev; });
    xpc_connection_resume(c);
    xpc_object_t m = xpc_dictionary_create_empty();
    xpc_dictionary_set_string(m, "op", "ping");
    xpc_dictionary_set_int64(m, "n", 21);
    xpc_object_t r = xpc_connection_send_message_with_reply_sync(c, m);
    check("the reply is a dictionary", r != NULL && xpc_get_type(r) == XPC_TYPE_DICTIONARY, 0);
    check("with the values the server set", xpc_dictionary_get_int64(r, "double") == 42 && strcmp(xpc_dictionary_get_string(r, "echo"), "ping") == 0, 0);
    check("the message sent is unchanged", xpc_dictionary_get_count(m) == 2 && xpc_dictionary_get_int64(m, "n") == 21, 0);
    xpc_dictionary_set_int64(m, "n", 500);
    xpc_object_t r2 = xpc_connection_send_message_with_reply_sync(c, m);
    check("a second message gets its own reply", xpc_dictionary_get_int64(r2, "double") == 1000, 0);
    check("the replies are different objects", r != r2, 0);
    xpc_release(r);
    xpc_release(r2);
    xpc_release(m);
    xpc_connection_cancel(c);
    return finish();
}
