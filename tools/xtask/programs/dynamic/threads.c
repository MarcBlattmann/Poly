/* POSIX threads in a dynamically linked program on Poly (roadmap Phase 4,
 * ADR 0049): pthread_create(3) and pthread_join(3), thread-local storage
 * (__thread, per thread), a mutex, a condition variable used as a
 * barrier, pthread_once(3) and thread-specific data with a destructor
 * (pthread_key_create(3)).
 *
 * Prints "dyn-threads: ok" and exits 0; on a failed check it prints what
 * failed and exits 1. Written for Poly from the man pages. */

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>

#define THREADS 4
#define ROUNDS 1000

static __thread long tls_counter;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t all_here = PTHREAD_COND_INITIALIZER;
static long total;
static int arrived;
static pthread_once_t once = PTHREAD_ONCE_INIT;
static int once_runs;
static pthread_key_t key;
static int destructor_runs;

static void init_once(void) {
    once_runs++;
}

static void destroy(void *value) {
    pthread_mutex_lock(&lock);
    destructor_runs++;
    pthread_mutex_unlock(&lock);
    free(value);
}

static void *worker(void *arg) {
    long id = (long)arg;
    pthread_once(&once, init_once);
    pthread_setspecific(key, malloc(16));
    for (int i = 0; i < ROUNDS; i++) {
        tls_counter++;
        pthread_mutex_lock(&lock);
        total++;
        pthread_mutex_unlock(&lock);
    }
    /* Every thread waits here until all have arrived. */
    pthread_mutex_lock(&lock);
    arrived++;
    pthread_cond_broadcast(&all_here);
    while (arrived < THREADS) {
        pthread_cond_wait(&all_here, &lock);
    }
    pthread_mutex_unlock(&lock);
    return (void *)(tls_counter + id);
}

int main(void) {
    pthread_t threads[THREADS];
    int failed = 0;
    if (pthread_key_create(&key, destroy) != 0) {
        printf("dyn-threads: FAIL pthread_key_create\n");
        return 1;
    }
    for (long i = 0; i < THREADS; i++) {
        if (pthread_create(&threads[i], NULL, worker, (void *)i) != 0) {
            printf("dyn-threads: FAIL pthread_create %ld\n", i);
            return 1;
        }
    }
    for (long i = 0; i < THREADS; i++) {
        void *result;
        if (pthread_join(threads[i], &result) != 0 || (long)result != ROUNDS + i) {
            printf("dyn-threads: FAIL thread %ld returned %ld\n", i, (long)result);
            failed = 1;
        }
    }
    if (total != THREADS * ROUNDS || tls_counter != 0) {
        printf("dyn-threads: FAIL total %ld, main thread's counter %ld\n", total, tls_counter);
        failed = 1;
    }
    if (once_runs != 1 || destructor_runs != THREADS) {
        printf("dyn-threads: FAIL pthread_once ran %d times, destructors %d\n", once_runs, destructor_runs);
        failed = 1;
    }
    if (failed) {
        return 1;
    }
    printf("dyn-threads: ok\n");
    return 0;
}
