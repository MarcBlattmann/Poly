/* pthread_attr_setstacksize / getstacksize: which sizes are accepted. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <unistd.h>

int test_main(void) {
    pthread_attr_t a;
    size_t sz = 0;
    check("attr_init", pthread_attr_init(&a) == 0, 0);
    check("getstacksize", pthread_attr_getstacksize(&a, &sz) == 0, 0);
    check("the default is 512 KiB", sz == 524288, (long long)sz);

    check("16384: 0", pthread_attr_setstacksize(&a, 16384) == 0, 0);
    pthread_attr_getstacksize(&a, &sz);
    check("... and read back", sz == 16384, (long long)sz);
    check("8192: 0", pthread_attr_setstacksize(&a, 8192) == 0, 0);
    check("32768: 0", pthread_attr_setstacksize(&a, 32768) == 0, 0);
    pthread_attr_getstacksize(&a, &sz);
    check("... read back 32768", sz == 32768, (long long)sz);
    check("81920 (5 * 16384): 0", pthread_attr_setstacksize(&a, 5 * 16384) == 0, 0);
    check("16 MiB: 0", pthread_attr_setstacksize(&a, 16 << 20) == 0, 0);
    pthread_attr_getstacksize(&a, &sz);
    check("... read back 16 MiB", sz == (16 << 20), (long long)sz);

    value("setstacksize 4096 (0 on arm64, EINVAL on x86_64)", pthread_attr_setstacksize(&a, 4096));
    check("12288 (a multiple of 4096): 0", pthread_attr_setstacksize(&a, 12288) == 0, 0);
    pthread_attr_setstacksize(&a, 16 << 20);
    int r = pthread_attr_setstacksize(&a, 1000);
    check("1000: EINVAL (22)", r == EINVAL, r);
    r = pthread_attr_setstacksize(&a, 0);
    check("0: EINVAL", r == EINVAL, r);
    r = pthread_attr_setstacksize(&a, 20000);
    check("20000 (not a multiple of 4096): EINVAL", r == EINVAL, r);
    r = pthread_attr_setstacksize(&a, 16384 + 1);
    check("16385: EINVAL", r == EINVAL, r);
    pthread_attr_getstacksize(&a, &sz);
    check("failures leave the old value (16 MiB)", sz == (16 << 20), (long long)sz);

    check("destroy", pthread_attr_destroy(&a) == 0, 0);
    r = pthread_attr_getstacksize(&a, &sz);
    check("an attribute after destroy: EINVAL", r == EINVAL, r);
    r = pthread_attr_setstacksize(&a, 32768);
    check("set after destroy: EINVAL", r == EINVAL, r);
    value("page size", getpagesize());
    return finish();
}
