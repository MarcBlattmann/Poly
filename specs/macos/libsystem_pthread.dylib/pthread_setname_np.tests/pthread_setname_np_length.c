/* pthread_setname_np: at most 63 bytes; a longer name is refused with -1
 * and errno ENAMETOOLONG (the call does not return the error number) and
 * leaves the old name. pthread_getname_np cuts to the buffer size. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <pthread.h>
#include <string.h>

int test_main(void) {
    char name[100], buf[100];
    for (int len = 62; len <= 66; len++) {
        memset(name, 'a' + (len % 20), (size_t)len);
        name[len] = 0;
        errno = 0;
        int r = pthread_setname_np(name);
        pthread_getname_np(pthread_self(), buf, sizeof buf);
        if (len <= 63) {
            check("up to 63 bytes: accepted", r == 0 && strlen(buf) == (size_t)len && memcmp(buf, name, (size_t)len) == 0, r);
        } else {
            check("64 bytes and more: returns -1", r == -1, r);
            check("64 bytes and more: errno ENAMETOOLONG (63)", errno == ENAMETOOLONG && errno == 63, errno);
            check("the previous 63 byte name is kept", strlen(buf) == 63, (long long)strlen(buf));
        }
    }
    pthread_setname_np("short-name");
    char small[8];
    memset(small, 'x', sizeof small);
    int r = pthread_getname_np(pthread_self(), small, 6);
    check("a buffer of 6 for a 10 byte name: returns 0", r == 0, r);
    check("the name is cut to 5 bytes and terminated", memcmp(small, "short", 5) == 0 && small[5] == 0, small[5]);
    check("bytes past the buffer are untouched", small[6] == 'x' && small[7] == 'x', small[6]);
    memset(small, 'x', sizeof small);
    pthread_getname_np(pthread_self(), small, 11);
    check_str("a buffer of exactly 11 holds the whole name", small, "short-name");
    memset(small, 'x', sizeof small);
    pthread_getname_np(pthread_self(), small, 1);
    check("a buffer of 1: just the NUL", small[0] == 0 && small[1] == 'x', small[1]);
    return finish();
}
