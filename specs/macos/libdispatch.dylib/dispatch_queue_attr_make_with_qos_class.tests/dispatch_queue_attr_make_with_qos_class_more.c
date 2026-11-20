/* The QoS class of a block's thread follows the queue. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <sys/qos.h>

int test_main(void) {
    dispatch_queue_t q = dispatch_queue_create("com.poly.util", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_UTILITY, 0));
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block qos_class_t seen = 0;
    dispatch_async(q, ^{ seen = qos_class_self(); dispatch_semaphore_signal(done); });
    gcd_wait(done);
    check("qos_class_self() inside a block of a utility queue is utility (17)", seen == QOS_CLASS_UTILITY, seen);

    dispatch_queue_t u = dispatch_queue_create("com.poly.ui", dispatch_queue_attr_make_with_qos_class(DISPATCH_QUEUE_SERIAL, QOS_CLASS_USER_INTERACTIVE, 0));
    dispatch_async(u, ^{ seen = qos_class_self(); dispatch_semaphore_signal(done); });
    gcd_wait(done);
    check("a user-interactive queue: 33", seen == QOS_CLASS_USER_INTERACTIVE, seen);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_BACKGROUND, 0), ^{ seen = qos_class_self(); dispatch_semaphore_signal(done); });
    gcd_wait(done);
    check("the background global queue: 9", seen == QOS_CLASS_BACKGROUND, seen);
    check("the main thread's own class is user-interactive (33)", qos_class_main() == QOS_CLASS_USER_INTERACTIVE, qos_class_main());
    return finish();
}
