/* dispatch_queue_set_specific(queue, key, value, destructor) and
 * dispatch_get_specific(key): per-queue values looked up by key address
 * from inside blocks; the destructor runs when the value is replaced or
 * the queue is freed. */
/* poly-mac: libsystem */
#include "poly_mgcd.h"

static int key1, key2;
static int destroyed_count;
static void *destroyed_value;
static dispatch_semaphore_t destroyed_sem;
static void destructor(void *v) {
    destroyed_value = v;
    destroyed_count++;
    dispatch_semaphore_signal(destroyed_sem);
}

int test_main(void) {
    destroyed_sem = dispatch_semaphore_create(0);
    dispatch_queue_t q = dispatch_queue_create("com.poly.specific", DISPATCH_QUEUE_SERIAL);
    check("outside any queue (main thread) a specific is NULL", dispatch_get_specific(&key1) == NULL, 0);
    check("dispatch_queue_get_specific reads from the queue directly: NULL at first", dispatch_queue_get_specific(q, &key1) == NULL, 0);
    dispatch_queue_set_specific(q, &key1, (void *)(long)100, destructor);
    dispatch_queue_set_specific(q, &key2, (void *)(long)200, NULL);
    check("dispatch_queue_get_specific gives the value", (long)dispatch_queue_get_specific(q, &key1) == 100, 0);
    check("the main thread is not on the queue: dispatch_get_specific is still NULL", dispatch_get_specific(&key1) == NULL, 0);

    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    __block long v1 = -1, v2 = -1, v3 = -1;
    static int key3;
    dispatch_async(q, ^{
        v1 = (long)dispatch_get_specific(&key1);
        v2 = (long)dispatch_get_specific(&key2);
        v3 = (long)dispatch_get_specific(&key3);
        dispatch_semaphore_signal(done);
    });
    gcd_wait(done);
    check("a block on the queue sees key1's value", v1 == 100, v1);
    check("and key2's", v2 == 200, v2);
    check("an unset key is NULL", v3 == 0, v3);

    dispatch_queue_set_specific(q, &key1, (void *)(long)101, destructor);
    check("replacing a value runs the old value's destructor (later, asynchronously) with the old value", gcd_wait(destroyed_sem) && (long)destroyed_value == 100, (long)destroyed_value);
    dispatch_queue_set_specific(q, &key1, NULL, NULL);
    check("setting NULL runs the destructor of the value removed", gcd_wait(destroyed_sem) && (long)destroyed_value == 101, (long)destroyed_value);
    check("and it reads NULL", dispatch_queue_get_specific(q, &key1) == NULL, 0);
    dispatch_queue_set_specific(q, &key1, (void *)(long)102, destructor);
    dispatch_release(q);
    check("releasing the queue runs the destructor of the value it holds", gcd_wait(destroyed_sem) && (long)destroyed_value == 102, (long)destroyed_value);
    check("three destructor calls in all", destroyed_count == 3, destroyed_count);
    return finish();
}
