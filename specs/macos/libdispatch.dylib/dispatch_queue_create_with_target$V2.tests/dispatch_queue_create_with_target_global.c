/* A queue aimed at a global queue runs on worker threads; aimed at the
 * main queue its blocks wait for the main thread. A NULL target means the
 * default global queue. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <sys/qos.h>

int test_main(void) {
    dispatch_queue_t g = dispatch_get_global_queue(QOS_CLASS_UTILITY, 0);
    dispatch_queue_t q = dispatch_queue_create_with_target("com.poly.to-global", DISPATCH_QUEUE_SERIAL, g);
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block int on_main = -1;
    __block qos_class_t qos = 0;
    dispatch_async(q, ^{
        on_main = gcd_on_main_thread();
        qos = qos_class_self();
        dispatch_semaphore_signal(done);
    });
    check("the block ran", gcd_wait(done), 0);
    check("not on the main thread", on_main == 0, on_main);
    value("QoS class of a block of a queue aimed at the utility global queue (17 = utility)", qos);

    dispatch_queue_t n = dispatch_queue_create_with_target("com.poly.null-target", DISPATCH_QUEUE_SERIAL, NULL);
    dispatch_async(n, ^{ on_main = gcd_on_main_thread(); dispatch_semaphore_signal(done); });
    check("a NULL target: the block runs", gcd_wait(done), 0);
    check("... off the main thread", on_main == 0, on_main);

    dispatch_queue_t m = dispatch_queue_create_with_target("com.poly.to-main", DISPATCH_QUEUE_SERIAL, dispatch_get_main_queue());
    __block int ran = 0;
    dispatch_async(m, ^{ ran = 1; dispatch_semaphore_signal(done); });
    check("aimed at the main queue: nothing runs while the main thread is not draining it", dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 200 * NSEC_PER_MSEC)) != 0 && ran == 0, ran);
    return finish();
}
