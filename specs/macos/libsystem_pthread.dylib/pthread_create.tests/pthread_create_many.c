/* pthread_create: many threads, concurrently alive; each gets its own
 * argument; their stacks are distinct. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>

#define N 64
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;
static int arrived, go;
static void *stack_addr[N];
static int arg_seen[N];

static void *start(void *arg) {
    int i = (int)(intptr_t)arg;
    char local;
    pthread_mutex_lock(&m);
    arg_seen[i] = i + 1;
    stack_addr[i] = &local;
    arrived++;
    pthread_cond_broadcast(&c);
    while (!go) pthread_cond_wait(&c, &m);
    pthread_mutex_unlock(&m);
    return (void *)(intptr_t)(i * 3);
}

int test_main(void) {
    pthread_t t[N];
    int fails = 0;
    for (int i = 0; i < N; i++) fails += pthread_create(&t[i], NULL, start, (void *)(intptr_t)i) != 0;
    check("64 creates return 0", fails == 0, fails);
    pthread_mutex_lock(&m);
    while (arrived < N) pthread_cond_wait(&c, &m);
    check("all 64 are alive at the same time (they wait for each other)", arrived == N, arrived);
    go = 1;
    pthread_cond_broadcast(&c);
    pthread_mutex_unlock(&m);
    int bad = 0;
    for (int i = 0; i < N; i++) {
        void *rv;
        pthread_join(t[i], &rv);
        if ((intptr_t)rv != i * 3) bad++;
        if (arg_seen[i] != i + 1) bad++;
    }
    check("each thread got its own argument and returned its own result", bad == 0, bad);
    int dup = 0;
    for (int i = 0; i < N; i++)
        for (int j = i + 1; j < N; j++)
            if ((char *)stack_addr[i] - (char *)stack_addr[j] < 4096 && (char *)stack_addr[j] - (char *)stack_addr[i] < 4096) dup++;
    check("stacks do not overlap (no two locals within 4096 bytes)", dup == 0, dup);
    return finish();
}
