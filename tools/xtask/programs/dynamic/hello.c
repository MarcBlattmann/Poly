/* A dynamically linked "hello world" for Poly's boot tests (roadmap Phase 4,
 * ADR 0049): built at test time with the host's C compiler, without
 * -static, so the kernel loads the program and the dynamic linker its
 * PT_INTERP names, and the dynamic linker loads the C library.
 *
 * It uses a few things that only work once the C library is relocated and
 * initialised: stdio, the heap, the environment, the auxiliary vector
 * (getauxval(3)) and its own arguments. Prints "dyn-hello: hello, world"
 * and exits 0; on a failed check it prints what failed and exits 1.
 * Written for Poly from the man pages. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/auxv.h>
#include <unistd.h>

static int failed;

static void check(int ok, const char *what) {
    if (!ok) {
        printf("dyn-hello: FAIL %s\n", what);
        failed = 1;
    }
}

int main(int argc, char **argv) {
    check(argc >= 1 && argv[0] != NULL && strstr(argv[0], "dyn-hello") != NULL, "argv[0] names the program");
    char *copy = strdup("hello, world");
    check(copy != NULL && strcmp(copy, "hello, world") == 0, "strdup");
    void *big = malloc(1 << 20);
    check(big != NULL, "a 1 MiB malloc (mmap)");
    if (big != NULL) {
        memset(big, 0x5a, 1 << 20);
    }
    free(big);
    check(setenv("POLY_DYN", "yes", 1) == 0 && strcmp(getenv("POLY_DYN"), "yes") == 0, "setenv/getenv");
    check(getauxval(AT_PAGESZ) == 4096 && sysconf(_SC_PAGESIZE) == 4096, "the page size");
    check(getauxval(AT_PHDR) != 0 && getauxval(AT_ENTRY) != 0, "AT_PHDR and AT_ENTRY");
    if (failed) {
        return 1;
    }
    printf("dyn-hello: %s\n", copy);
    free(copy);
    return 0;
}
