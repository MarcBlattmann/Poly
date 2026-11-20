/* arc4random_buf is usable after fork in the child, and the child's output
 * differs from the parent's. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <spawn.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

int test_main(void) {
    int fds[2];
    if (pipe(fds) != 0) return 1;
    /* The generator is seeded by the first draw, before the fork. */
    unsigned char before[16];
    arc4random_buf(before, sizeof before);
    pid_t pid = fork();
    if (pid == 0) {
        unsigned char c[16];
        arc4random_buf(c, sizeof c);
        write(fds[1], c, sizeof c);
        _exit(0);
    }
    unsigned char p[16], c[16];
    arc4random_buf(p, sizeof p);
    int st = 0;
    waitpid(pid, &st, 0);
    ssize_t n = read(fds[0], c, sizeof c);
    check("the child wrote 16 bytes", n == 16 && WIFEXITED(st), (long long)n);
    check("the child's bytes differ from the parent's (no shared state after fork)", memcmp(c, p, 16) != 0, 0);
    check("and from the bytes drawn before the fork", memcmp(c, before, 16) != 0, 0);
    return finish();
}
