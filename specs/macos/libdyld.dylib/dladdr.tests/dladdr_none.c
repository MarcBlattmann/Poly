/* dladdr for addresses in no image: 0, and the output is not touched. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    Dl_info i;
    int local = 0;
    char *heap = malloc(16);
    void *addrs[] = {NULL, (void *)0x1000, &local, heap};
    const char *names[] = {"NULL", "0x1000", "stack", "heap"};
    for (int k = 0; k < 4; k++) {
        memset(&i, 0x5a, sizeof i);
        int r = dladdr(addrs[k], &i);
        check(names[k], r == 0, r);
        unsigned char *b = (unsigned char *)&i;
        int touched = 0;
        for (unsigned j = 0; j < sizeof i; j++) touched += b[j] != 0x5a;
        check("... and Dl_info is left alone", touched == 0, touched);
    }
    value("address -1 (it lies in a mapped system page: nonzero)", dladdr((void *)~(uintptr_t)0, &i));
    value("sizeof(Dl_info)", sizeof(Dl_info));
    free(heap);
    return finish();
}
