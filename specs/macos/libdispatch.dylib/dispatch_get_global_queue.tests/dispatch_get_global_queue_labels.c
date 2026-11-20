/* dispatch_get_global_queue: one queue per quality of service; the old
 * priority constants map to QoS classes; unknown arguments give NULL. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <sys/qos.h>

int test_main(void) {
    check_str("QOS_CLASS_DEFAULT", dispatch_queue_get_label(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0)), "com.apple.root.default-qos");
    check_str("QOS_CLASS_USER_INTERACTIVE", dispatch_queue_get_label(dispatch_get_global_queue(QOS_CLASS_USER_INTERACTIVE, 0)), "com.apple.root.user-interactive-qos");
    check_str("QOS_CLASS_USER_INITIATED", dispatch_queue_get_label(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0)), "com.apple.root.user-initiated-qos");
    check_str("QOS_CLASS_UTILITY", dispatch_queue_get_label(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0)), "com.apple.root.utility-qos");
    check_str("QOS_CLASS_BACKGROUND", dispatch_queue_get_label(dispatch_get_global_queue(QOS_CLASS_BACKGROUND, 0)), "com.apple.root.background-qos");
    check_str("DISPATCH_QUEUE_PRIORITY_HIGH is the user-initiated queue", dispatch_queue_get_label(dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_HIGH, 0)), "com.apple.root.user-initiated-qos");
    check_str("DISPATCH_QUEUE_PRIORITY_DEFAULT is the default queue", dispatch_queue_get_label(dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0)), "com.apple.root.default-qos");
    check_str("DISPATCH_QUEUE_PRIORITY_LOW is the utility queue", dispatch_queue_get_label(dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_LOW, 0)), "com.apple.root.utility-qos");
    check_str("DISPATCH_QUEUE_PRIORITY_BACKGROUND is the background queue", dispatch_queue_get_label(dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_BACKGROUND, 0)), "com.apple.root.background-qos");
    check_str("0 (QOS_CLASS_UNSPECIFIED) is the default queue", dispatch_queue_get_label(dispatch_get_global_queue(0, 0)), "com.apple.root.default-qos");
    check("the same queue on every call", dispatch_get_global_queue(QOS_CLASS_UTILITY, 0) == dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), 0);
    check("the queues of different classes differ", dispatch_get_global_queue(QOS_CLASS_UTILITY, 0) != dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), 0);
    check("an unknown class (12345): NULL", dispatch_get_global_queue(12345, 0) == NULL, 0);
    check("nonzero flags: NULL", dispatch_get_global_queue(QOS_CLASS_DEFAULT, 1) == NULL, 0);
    value("QOS_CLASS_USER_INTERACTIVE", QOS_CLASS_USER_INTERACTIVE);
    value("QOS_CLASS_USER_INITIATED", QOS_CLASS_USER_INITIATED);
    value("QOS_CLASS_DEFAULT", QOS_CLASS_DEFAULT);
    value("QOS_CLASS_UTILITY", QOS_CLASS_UTILITY);
    value("QOS_CLASS_BACKGROUND", QOS_CLASS_BACKGROUND);
    return finish();
}
