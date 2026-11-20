/* poly-mac: libsystem
 * xpc_connection_create_mach_service with XPC_CONNECTION_MACH_SERVICE_LISTENER for a name no job declares (also one this
 * process checked in with bootstrap_check_in): the listener object is made, but when resumed its handler gets
 * XPC_ERROR_CONNECTION_INVALID and no connection ever arrives; a client of that name gets nothing useful either. */
#include "poly_mapi.h"
int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("poly.xcm2", NULL);
    char name[100], name2[100];
    service_name(name, sizeof name, "xcm_listen");
    service_name(name2, sizeof name2, "xcm_listen_ci");
    xpc_connection_t l = xpc_connection_create_mach_service(name, q, XPC_CONNECTION_MACH_SERVICE_LISTENER);
    check("a connection object", l != NULL && xpc_get_type(l) == XPC_TYPE_CONNECTION, 0);
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block xpc_object_t first = NULL;
    __block int connections = 0;
    xpc_connection_set_event_handler(l, ^(xpc_object_t ev) {
        if (xpc_get_type(ev) == XPC_TYPE_CONNECTION) connections++;
        if (!first) first = ev;
        dispatch_semaphore_signal(sem);
    });
    xpc_connection_resume(l);
    check("the handler is called", xpc_wait(sem, 5), 0);
    check("with XPC_ERROR_CONNECTION_INVALID", first == XPC_ERROR_CONNECTION_INVALID, 0);
    check("and no connection came", connections == 0, connections);
    mach_port_t recv = 0;
    (void)!bootstrap_check_in(bootstrap_port, name2, &recv);
    xpc_connection_t l2 = xpc_connection_create_mach_service(name2, q, XPC_CONNECTION_MACH_SERVICE_LISTENER);
    dispatch_semaphore_t sem2 = dispatch_semaphore_create(0);
    __block xpc_object_t first2 = NULL;
    xpc_connection_set_event_handler(l2, ^(xpc_object_t ev) {
        if (!first2) first2 = ev;
        dispatch_semaphore_signal(sem2);
    });
    xpc_connection_resume(l2);
    check("a checked-in name does not make a listener valid", xpc_wait(sem2, 5) && first2 == XPC_ERROR_CONNECTION_INVALID, 0);
    return finish();
}
