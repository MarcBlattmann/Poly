/* Static glibc "hello world". Its trace is the C library's startup sequence,
 * which the work queue (`pipeline next`) uses to order syscalls by first use.
 * Built only when the host has a static C library. */
#include <stdio.h>

int main(void) {
    printf("hello, world\n");
    return 0;
}
