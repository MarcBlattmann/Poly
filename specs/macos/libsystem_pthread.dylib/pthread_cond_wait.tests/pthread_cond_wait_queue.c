/* pthread_cond_wait as a producer/consumer queue with two conditions. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>

#define CAP 4
#define ITEMS 5000
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t not_empty = PTHREAD_COND_INITIALIZER, not_full = PTHREAD_COND_INITIALIZER;
static int q[CAP], head, count;
static long sum_consumed;
static int out_of_order;

static void *producer(void *arg) {
    (void)arg;
    for (int i = 1; i <= ITEMS; i++) {
        pthread_mutex_lock(&m);
        while (count == CAP) pthread_cond_wait(&not_full, &m);
        q[(head + count) % CAP] = i;
        count++;
        pthread_cond_signal(&not_empty);
        pthread_mutex_unlock(&m);
    }
    return NULL;
}
static void *consumer(void *arg) {
    (void)arg;
    int last = 0;
    for (int i = 1; i <= ITEMS; i++) {
        pthread_mutex_lock(&m);
        while (count == 0) pthread_cond_wait(&not_empty, &m);
        int v = q[head];
        head = (head + 1) % CAP;
        count--;
        pthread_cond_signal(&not_full);
        pthread_mutex_unlock(&m);
        if (v != last + 1) out_of_order++;
        last = v;
        sum_consumed += v;
    }
    return NULL;
}

int test_main(void) {
    pthread_t p, c;
    pthread_create(&c, NULL, consumer, NULL);
    pthread_create(&p, NULL, producer, NULL);
    pthread_join(p, NULL);
    pthread_join(c, NULL);
    check("5000 items passed through a 4 slot queue in order", out_of_order == 0, out_of_order);
    check("none lost or duplicated (sum 12502500)", sum_consumed == 12502500L, sum_consumed);
    check("the queue is empty", count == 0, count);
    return finish();
}
