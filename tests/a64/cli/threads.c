/* (c) Threads, signals and memory mappings for Poly's arm64 path (ADR
 * 0257 addendum), linked the ordinary way against the SDK. Each check
 * prints "ok <what>" or "not ok <what>"; the last line counts failures.
 * Written for Poly. */
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static int failures;
static void check(const char *what, int ok) {
    printf("%s %s\n", ok ? "ok" : "not ok", what);
    if (!ok) failures++;
}

/* ------------------------------------------------------------ threads */

#define THREADS 4
#define ROUNDS 20000

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static long counter;
static _Thread_local long mine;
static pthread_key_t key;

static void *count_up(void *arg) {
    long id = (long)arg;
    mine = id * 1000;
    pthread_setspecific(key, (void *)(id + 1));
    for (int i = 0; i < ROUNDS; i++) {
        pthread_mutex_lock(&lock);
        counter++;
        pthread_mutex_unlock(&lock);
        mine++;
    }
    /* Every thread has its own errno and its own __thread variable. */
    errno = 100 + (int)id;
    sched_yield();
    int ok = errno == 100 + (int)id && mine == id * 1000 + ROUNDS && pthread_getspecific(key) == (void *)(id + 1);
    return (void *)(long)(ok ? id : -1);
}

static pthread_mutex_t qlock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t qcond = PTHREAD_COND_INITIALIZER;
static int queue[16], qhead, qtail, qdone;

static void *produce(void *arg) {
    (void)arg;
    for (int i = 1; i <= 200; i++) {
        pthread_mutex_lock(&qlock);
        while (qtail - qhead == 16) pthread_cond_wait(&qcond, &qlock);
        queue[qtail++ % 16] = i;
        pthread_cond_broadcast(&qcond);
        pthread_mutex_unlock(&qlock);
    }
    pthread_mutex_lock(&qlock);
    qdone = 1;
    pthread_cond_broadcast(&qcond);
    pthread_mutex_unlock(&qlock);
    return NULL;
}

static int once_runs;
static void once_body(void) { once_runs++; }
static void *call_once(void *arg) {
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_once(&once, once_body);
    return arg;
}

static void test_threads(void) {
    pthread_key_create(&key, NULL);
    pthread_t t[THREADS];
    for (long i = 0; i < THREADS; i++) pthread_create(&t[i], NULL, count_up, (void *)i);
    int results_ok = 1;
    for (long i = 0; i < THREADS; i++) {
        void *r;
        pthread_join(t[i], &r);
        if ((long)r != i) results_ok = 0;
    }
    check("four threads share a mutex-guarded counter", counter == THREADS * ROUNDS);
    check("each thread has its own errno, __thread variable and key value", results_ok);
    check("the main thread's own values are untouched", mine == 0 && pthread_getspecific(key) == NULL);

    pthread_t p;
    pthread_create(&p, NULL, produce, NULL);
    long sum = 0;
    int got = 0;
    pthread_mutex_lock(&qlock);
    for (;;) {
        while (qhead == qtail && !qdone) pthread_cond_wait(&qcond, &qlock);
        if (qhead == qtail && qdone) break;
        sum += queue[qhead++ % 16];
        got++;
        pthread_cond_broadcast(&qcond);
    }
    pthread_mutex_unlock(&qlock);
    pthread_join(p, NULL);
    check("a condition variable hands 200 items over", got == 200 && sum == 200 * 201 / 2);

    pthread_t o[3];
    for (long i = 0; i < 3; i++) pthread_create(&o[i], NULL, call_once, NULL);
    for (int i = 0; i < 3; i++) pthread_join(o[i], NULL);
    call_once(NULL);
    check("pthread_once runs once", once_runs == 1);
    check("pthread_self differs between threads", !pthread_equal(pthread_self(), t[0]));
}

/* ------------------------------------------------------------- memory */

static void test_mmap(void) {
    size_t page = (size_t)getpagesize();
    char *a = mmap(NULL, 4 * page, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    int ok = a != MAP_FAILED;
    if (ok) {
        for (size_t i = 0; i < 4 * page; i++) ok &= a[i] == 0;
        memset(a, 'x', 4 * page);
        ok &= a[4 * page - 1] == 'x' && munmap(a, 4 * page) == 0;
    }
    check("anonymous mmap is zeroed, writable, unmapped", ok);

    const char *path = "/tmp/poly-a64-map.txt";
    int fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    const char text[] = "mapped from a file\n";
    write(fd, text, sizeof text - 1);
    char *m = mmap(NULL, page, PROT_READ, MAP_PRIVATE, fd, 0);
    check("a file maps and reads back", m != MAP_FAILED && memcmp(m, text, sizeof text - 1) == 0 && m[sizeof text] == 0);
    if (m != MAP_FAILED) munmap(m, page);
    close(fd);
    unlink(path);

    check("getpagesize is 16 KiB", page == 16384);
}

/* ------------------------------------------------------------ signals */

static volatile sig_atomic_t usr1_runs, usr2_runs;
static volatile int usr1_signo, usr1_pid;
static void on_usr1(int sig, siginfo_t *info, void *uc) {
    (void)uc;
    usr1_runs++;
    usr1_signo = sig == SIGUSR1 ? info->si_signo : -1;
    usr1_pid = info->si_pid;
}
static void on_usr2(int sig) { usr2_runs += sig == SIGUSR2; }

static sigjmp_buf recover;
static volatile int fault_signo, fault_code;
static volatile void *fault_addr;
static void on_fault(int sig, siginfo_t *info, void *uc) {
    (void)uc;
    fault_signo = sig;
    fault_code = info->si_code;
    fault_addr = info->si_addr;
    siglongjmp(recover, 1);
}

static void test_signals(void) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_usr1;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGUSR1, &sa, NULL);
    /* Values live in registers across the handler. */
    volatile long a = 6, b = 7;
    long before = a * b;
    raise(SIGUSR1);
    check("raise runs a SA_SIGINFO handler before it returns",
          usr1_runs == 1 && usr1_signo == SIGUSR1 && usr1_pid == getpid() && a * b == before);

    signal(SIGUSR2, on_usr2);
    sigset_t set, old;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR2);
    sigprocmask(SIG_BLOCK, &set, &old);
    kill(getpid(), SIGUSR2);
    int while_blocked = usr2_runs;
    sigprocmask(SIG_SETMASK, &old, NULL);
    check("a blocked signal waits until it is unblocked", while_blocked == 0 && usr2_runs == 1);

    struct sigaction fa;
    memset(&fa, 0, sizeof fa);
    fa.sa_sigaction = on_fault;
    fa.sa_flags = SA_SIGINFO;
    sigaction(SIGSEGV, &fa, NULL);
    sigaction(SIGBUS, &fa, NULL);
    if (sigsetjmp(recover, 1) == 0) {
        *(volatile int *)8 = 1;
        check("a write to address 8 does not go on", 0);
    }
    check("a write to address 8 is SIGSEGV with SEGV_ACCERR and si_addr 8",
          fault_signo == SIGSEGV && fault_code == SEGV_ACCERR && fault_addr == (void *)8);

    size_t page = (size_t)getpagesize();
    char *p = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    p[0] = 1;
    mprotect(p, page, PROT_READ);
    fault_signo = 0;
    if (sigsetjmp(recover, 1) == 0) {
        p[40] = 2;
        check("a write to a read-only page does not go on", 0);
    }
    check("a write to a read-only page is SIGBUS at its address", fault_signo == SIGBUS && fault_addr == p + 40);
    /* The handler left by siglongjmp: the mask is back, so a second fault
     * runs it again. */
    fault_signo = 0;
    if (sigsetjmp(recover, 1) == 0) {
        *(volatile int *)16 = 1;
    }
    check("siglongjmp restores the mask: a second fault is handled", fault_signo == SIGSEGV && fault_addr == (void *)16);
    munmap(p, page);
}

int main(void) {
    printf("threads: arm64 threads, signals and mappings\n");
    test_threads();
    test_mmap();
    test_signals();
    printf("threads: done, %d failed\n", failures);
    return failures;
}
