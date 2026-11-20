/* pthread_threadid_np: a forked child's main thread has its own id. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <pthread.h>
#include <stdint.h>
#include <sys/wait.h>
#include <unistd.h>

int test_main(void) {
    uint64_t parent;
    pthread_threadid_np(NULL, &parent);
    int fds[2];
    if (pipe(fds) != 0) return 1;
    pid_t pid = fork();
    if (pid == 0) {
        uint64_t mine = 0;
        int r = pthread_threadid_np(NULL, &mine);
        uint64_t out[2] = {mine, (uint64_t)r};
        write(fds[1], out, sizeof out);
        _exit(0);
    }
    uint64_t got[2] = {0, 99};
    int st = 0;
    waitpid(pid, &st, 0);
    ssize_t n = read(fds[0], got, sizeof got);
    check("the child reported", n == (ssize_t)sizeof got && WIFEXITED(st), (long long)n);
    check("its call returned 0", got[1] == 0, (long long)got[1]);
    check("its id is nonzero and not its parent's thread id", got[0] != 0 && got[0] != parent, (long long)got[0]);
    return finish();
}
