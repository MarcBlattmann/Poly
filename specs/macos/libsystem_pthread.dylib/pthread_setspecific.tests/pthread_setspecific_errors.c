/* pthread_setspecific with a key that was deleted or never made: EINVAL. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>

int test_main(void) {
    pthread_key_t k;
    pthread_key_create(&k, NULL);
    pthread_setspecific(k, (void *)5);
    pthread_key_delete(k);
    int r = pthread_setspecific(k, (void *)6);
    check("set on a deleted key: EINVAL (22)", r == EINVAL, r);
    check("get on a deleted key: NULL", pthread_getspecific(k) == NULL, 0);
    r = pthread_key_delete(k);
    check("deleting a deleted key: EINVAL", r == EINVAL, r);
    pthread_key_t never = 100000;
    r = pthread_setspecific(never, (void *)1);
    check("set on a key number far out of range: EINVAL", r == EINVAL, r);
    r = pthread_key_delete(never);
    check("delete of a key number far out of range: EINVAL", r == EINVAL, r);

    pthread_key_t k2;
    pthread_key_create(&k2, NULL);
    check("the failures did not break key creation", pthread_getspecific(k2) == NULL, 0);
    check("set on the new key works", pthread_setspecific(k2, (void *)8) == 0, 0);
    pthread_key_delete(k2);
    return finish();
}
