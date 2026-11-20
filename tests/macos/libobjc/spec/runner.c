/* Runs the x86-64 builds of the libobjc visible spec tests on Poly, one
 * after another, as children (posix_spawn, wait4: ADR 0246): it reads the
 * names from /etc/objc-tests.txt, prints "test NAME" before each and
 * "exit NAME STATUS" after it (the wait4 status word), then "runner: done".
 * Raw system calls only. Written for Poly. */
typedef unsigned long u64;

static long sys(long n, long a, long b, long c, long d, long e) {
    long r;
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    __asm__ volatile("syscall" : "=a"(r) : "a"(0x2000000L | n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8)
                     : "rcx", "r11", "memory", "cc");
    return r;
}

static unsigned long len(const char *s) {
    unsigned long n = 0;
    while (s[n]) n++;
    return n;
}
static void out(const char *s) { sys(4, 1, (long)s, (long)len(s), 0, 0); }
static void out_num(long v) {
    char b[24];
    int i = 23;
    b[i] = 0;
    if (v == 0) b[--i] = '0';
    while (v > 0) {
        b[--i] = (char)('0' + v % 10);
        v /= 10;
    }
    out(b + i);
}

static char list[16384];
static char path[256];

int start(void) __asm__("start");
int start(void) {
    long fd = sys(5, (long)"/etc/objc-tests.txt", 0, 0, 0, 0);
    long n = sys(3, fd, (long)list, sizeof list - 1, 0, 0);
    if (n < 0) n = 0;
    list[n] = 0;
    char *p = list;
    while (*p) {
        char *name = p;
        while (*p && *p != '\n') p++;
        if (*p) *p++ = 0;
        if (!*name) continue;
        const char *dir = "/bin/objc/";
        unsigned long i = 0;
        for (const char *q = dir; *q; q++) path[i++] = *q;
        for (const char *q = name; *q && i < sizeof path - 1; q++) path[i++] = *q;
        path[i] = 0;
        out("test ");
        out(name);
        out("\n");
        char *argv[] = {path, 0};
        char *envp[] = {0};
        int pid = 0;
        long r = sys(244, (long)&pid, (long)path, 0, (long)argv, (long)envp);
        int status = -1;
        if (r == 0) sys(7, pid, (long)&status, 0, 0, 0);
        out("exit ");
        out(name);
        out(" ");
        out_num(status < 0 ? 99999 : status);
        out("\n");
    }
    out("runner: done\n");
    sys(1, 0, 0, 0, 0, 0);
    for (;;) {
    }
}
