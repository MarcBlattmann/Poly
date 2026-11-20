/* dispatch_queue_attr_make_with_qos_class: the attribute carries the
 * serial/concurrent choice and a QoS class with a relative priority (0 or
 * negative, down to -15); the queue made from it reports them. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <sys/qos.h>

int test_main(void) {
    int rp = 99;
    dispatch_queue_attr_t a = dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_UTILITY, -3);
    check("a serial attribute with a class is not NULL", a != NULL, 0);
    dispatch_queue_t q = dispatch_queue_create("com.poly.qos", a);
    check("the queue's class is utility (17)", dispatch_queue_get_qos_class(q, &rp) == QOS_CLASS_UTILITY, 0);
    check("its relative priority is -3", rp == -3, rp);

    a = dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_CONCURRENT, QOS_CLASS_USER_INTERACTIVE, 0);
    q = dispatch_queue_create("com.poly.qos2", a);
    check("user-interactive concurrent queue: class 33", dispatch_queue_get_qos_class(q, &rp) == QOS_CLASS_USER_INTERACTIVE && rp == 0, rp);

    for (int rel = 0; rel >= -15; rel -= 5) {
        a = dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_BACKGROUND, rel);
        q = dispatch_queue_create("com.poly.rel", a);
        int got = 99;
        dispatch_queue_get_qos_class(q, &got);
        check("relative priority 0, -5, -10, -15 are kept", got == rel, got);
    }
    check("a positive relative priority: NULL", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_DEFAULT, 1) == NULL, 0);
    check("below -15: NULL", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_DEFAULT, -16) == NULL, 0);
    check("an unknown class (12345): NULL", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, 12345, 0) == NULL, 0);
    check("QOS_CLASS_UNSPECIFIED is accepted", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_UNSPECIFIED, 0) != NULL, 0);
    check("the same arguments give the same attribute (a shared constant)", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_UTILITY, -3) == dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_UTILITY, -3), 0);
    check("a different class gives a different attribute", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_UTILITY, 0) != dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_BACKGROUND, 0), 0);
    return finish();
}
