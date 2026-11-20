/* While the main thread sits in dispatch_main, blocks on other queues run
 * on their own threads, and work they submit to the main queue still runs
 * on the main queue (not necessarily on the original main thread). */
/* poly-mac: libsystem */
#include "poly_mgcd.h"
#include <stdlib.h>

static int worker_on_main = -1;
static long long worker_tid, main_tid;

int test_main(void) {
    main_tid = gcd_current_thread_id();
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
        worker_on_main = gcd_on_main_thread();
        worker_tid = gcd_current_thread_id();
        dispatch_async(dispatch_get_main_queue(), ^{
            check("a global queue block ran off the main thread while main was in dispatch_main", worker_on_main == 0 && worker_tid != main_tid, worker_on_main);
            check_str("the block it sent to the main queue runs on the main queue", dispatch_queue_get_label(DISPATCH_CURRENT_QUEUE_LABEL), "com.apple.main-thread");
            value("... on the process's original main thread (pthread_main_np): blocks that arrive after dispatch_main's first drain run on another thread", gcd_on_main_thread());
            value("... and its thread id is the main thread's", gcd_current_thread_id() == main_tid);
            exit(finish());
        });
    });
    dispatch_main();
}
