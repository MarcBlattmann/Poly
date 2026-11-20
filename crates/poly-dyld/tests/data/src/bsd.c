/* The BSD layer on Poly (ADR 0246): a program linked through Poly's dyld
 * against the libSystem stand-in that makes the raw calls a command-line
 * program's start-up and work need: files, directories, mmap, sysctl, ids,
 * signals, a pipe, and a child started with posix_spawn and reaped with
 * wait4. Prints one line per check; main returns the number of failures.
 * Run as "bsd child" it is the child: it writes to fd 9 and exits 7.
 * Written for Poly; numbers from the macOS SDK headers named. */
#include "sys.h"

extern long poly_write(int fd, const char *buf, long len);

typedef unsigned long u64;
typedef unsigned int u32;

/* <sys/syscall.h> */
#define SYS_read 3
#define SYS_open 5
#define SYS_close 6
#define SYS_wait4 7
#define SYS_getpid 20
#define SYS_kill 37
#define SYS_pipe 42
#define SYS_sigaction 46
#define SYS_sigprocmask 48
#define SYS_sigpending 52
#define SYS_munmap 73
#define SYS_fcntl 92
#define SYS_pread 153
#define SYS_csops 169
#define SYS_mmap 197
#define SYS_lseek 199
#define SYS_sysctl 202
#define SYS_posix_spawn 244
#define SYS_sysctlbyname 274
#define SYS_issetugid 327
#define SYS_stat64 338
#define SYS_fstat64 339
#define SYS_getdirentries64 344
#define SYS_getentropy 500

struct ret { long v, v2; int err; };

static struct ret sys6(long n, long a, long b, long c, long d, long e, long f) {
    struct ret r;
    long rax, rdx;
    unsigned char cf;
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    __asm__ volatile("syscall; setc %2"
                     : "=a"(rax), "=d"(rdx), "=r"(cf)
                     : "a"(0x2000000L | n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory", "cc");
    r.v = rax;
    r.v2 = rdx;
    r.err = cf ? (int)rax : 0;
    return r;
}
#define sys(n, ...) sys6_(n, __VA_ARGS__, 0, 0, 0, 0, 0, 0)
#define sys6_(n, a, b, c, d, e, f, ...) sys6(n, (long)(a), (long)(b), (long)(c), (long)(d), (long)(e), (long)(f))

static int failures;
static void say(const char *s) { poly_write(1, s, (long)poly_strlen(s)); }
static void check(const char *what, int ok) {
    say(ok ? "ok " : "not ok ");
    say(what);
    say("\n");
    if (!ok) failures++;
}
static int eq(const char *a, const char *b, unsigned long n) {
    for (unsigned long i = 0; i < n; i++)
        if (a[i] != b[i]) return 0;
    return 1;
}

static char buf[4096];

static void files(void) {
    struct ret r = sys(SYS_open, "/etc/mbsd.txt", 0, 0);
    check("open a file", r.err == 0 && r.v >= 3);
    long fd = r.v;
    r = sys(SYS_read, fd, buf, sizeof buf);
    check("read it", r.err == 0 && r.v == 20 && eq(buf, "poly reads macos io\n", 20));
    check("read at the end is 0", sys(SYS_read, fd, buf, 10).v == 0);
    check("lseek", sys(SYS_lseek, fd, 5, 0).v == 5);
    r = sys(SYS_pread, fd, buf, 5, 11);
    check("pread", r.v == 5 && eq(buf, "macos", 5));
    r = sys(SYS_fstat64, fd, buf);
    check("fstat64: a regular file of 20 bytes",
          r.err == 0 && (*(unsigned short *)(buf + 4) & 0170000) == 0100000 && *(long *)(buf + 96) == 20);
    r = sys(SYS_fcntl, fd, 50, buf); /* F_GETPATH */
    check("fcntl F_GETPATH", r.err == 0 && eq(buf, "/etc/mbsd.txt", 14));
    r = sys(SYS_fcntl, fd, 0, 20); /* F_DUPFD */
    check("fcntl F_DUPFD", r.v == 20);
    check("close", sys(SYS_close, fd).err == 0 && sys(SYS_close, 20).err == 0);
    check("close twice is EBADF", sys(SYS_close, fd).err == 9);
    check("open a missing file is ENOENT", sys(SYS_open, "/etc/nope", 0, 0).err == 2);
    r = sys(SYS_stat64, "/usr/lib", buf);
    check("stat64 a directory", r.err == 0 && (*(unsigned short *)(buf + 4) & 0170000) == 0040000);
}

static void directory(void) {
    struct ret r = sys(SYS_open, "/usr/lib/poly-dyld-test", 0x100000, 0); /* O_DIRECTORY */
    long fd = r.v;
    int seen_greet = 0, seen_dot = 0, calls = 0;
    u64 base = 1;
    for (;;) {
        r = sys(SYS_getdirentries64, fd, buf, 256, &base);
        if (r.err || r.v == 0 || ++calls > 50) break;
        for (long at = 0; at < r.v;) {
            unsigned short reclen = *(unsigned short *)(buf + at + 16);
            const char *name = buf + at + 21;
            if (eq(name, "libgreet.dylib", 15) && buf[at + 20] == 8) seen_greet = 1;
            if (eq(name, ".", 2) && buf[at + 20] == 4) seen_dot = 1;
            at += reclen;
        }
    }
    check("getdirentries64 lists a directory", r.err == 0 && seen_greet && seen_dot);
    check("basep at the end", base == 0x7fffffff);
    sys(SYS_close, fd);
}

static void memory(void) {
    struct ret r = sys(SYS_mmap, 0, 3 * 4096, 3, 0x1002, -1, 0); /* MAP_ANON|MAP_PRIVATE */
    char *p = (char *)r.v;
    check("mmap anonymous memory", r.err == 0 && r.v != 0 && (r.v & 4095) == 0);
    int zero = 1;
    for (int i = 0; i < 3 * 4096; i += 512) zero &= p[i] == 0;
    p[0] = 'x';
    p[3 * 4096 - 1] = 'y';
    check("it is zero filled and writable", zero && p[0] == 'x' && p[3 * 4096 - 1] == 'y');
    check("munmap", sys(SYS_munmap, p, 3 * 4096).err == 0);
    long fd = sys(SYS_open, "/etc/mbsd.txt", 0, 0).v;
    r = sys(SYS_mmap, 0, 20, 1, 2, fd, 0); /* PROT_READ, MAP_PRIVATE */
    check("mmap a file", r.err == 0 && eq((char *)r.v, "poly reads macos io\n", 20));
    sys(SYS_close, fd);
    check("munmap unaligned is EINVAL", sys(SYS_munmap, 4097, 4096).err == 22);
}

static void system_info(void) {
    int mib[2] = {1, 1}; /* CTL_KERN, KERN_OSTYPE */
    u64 len = sizeof buf;
    struct ret r = sys(SYS_sysctl, mib, 2, buf, &len, 0, 0);
    check("sysctl kern.ostype", r.err == 0 && len == 7 && eq(buf, "Darwin", 7));
    u64 page = 0;
    len = 8;
    r = sys(SYS_sysctlbyname, "hw.pagesize", 11, &page, &len, 0, 0);
    check("sysctlbyname hw.pagesize", r.err == 0 && len == 8 && page == 4096);
    int ncpu = 0;
    len = 4;
    r = sys(SYS_sysctlbyname, "hw.ncpu", 7, &ncpu, &len, 0, 0);
    check("sysctlbyname hw.ncpu", r.err == 0 && ncpu >= 1);
    check("getpid", sys(SYS_getpid, 0).v > 0);
    check("issetugid", sys(SYS_issetugid, 0).v == 0);
    u32 flags = 0;
    check("csops status", sys(SYS_csops, 0, 0, &flags, 4).err == 0 && (flags & 1));
    char e[32] = {0};
    check("getentropy", sys(SYS_getentropy, e, 32).err == 0 && (e[0] | e[1] | e[2] | e[3]));
}

static void signals(void) {
    u32 set = 1u << 29; /* SIGUSR1 30 */
    check("sigprocmask blocks SIGUSR1", sys(SYS_sigprocmask, 1, &set, 0).err == 0);
    long me = sys(SYS_getpid, 0).v;
    check("kill itself with a blocked signal", sys(SYS_kill, me, 30).err == 0);
    u32 pending = 0;
    sys(SYS_sigpending, &pending);
    check("it is pending", pending == set);
    long ign[3] = {1, 0, 0}; /* SIG_IGN */
    check("sigaction SIG_IGN for SIGTERM", sys(SYS_sigaction, 15, ign, 0).err == 0);
    check("an ignored SIGTERM does nothing", sys(SYS_kill, me, 15).err == 0);
    check("sigaction of SIGKILL is EINVAL", sys(SYS_sigaction, 9, ign, 0).err == 22);
}

static void child_process(char *self) {
    struct ret p = sys(SYS_pipe, 0);
    check("pipe", p.err == 0);
    /* The child inherits the write end as fd 9 too. */
    sys(90, p.v2, 9); /* dup2 */
    char *argv[] = {self, "child", 0};
    char *envp[] = {"MBSD=1", 0};
    int pid = 0;
    struct ret r = sys(SYS_posix_spawn, &pid, self, 0, argv, envp);
    check("posix_spawn", r.err == 0 && pid > 0);
    sys(SYS_close, 9);
    sys(SYS_close, p.v2);
    r = sys(SYS_read, p.v, buf, 64);
    check("the child writes through the inherited pipe", r.v == 9 && eq(buf, "child hi\n", 9));
    int status = 0;
    r = sys(SYS_wait4, pid, &status, 0, 0);
    check("wait4 reaps it with exit status 7", r.v == pid && status == 0x700);
    check("then ECHILD", sys(SYS_wait4, -1, &status, 0, 0).err == 10);
    check("posix_spawn of a missing file is ENOENT", sys(SYS_posix_spawn, &pid, "/bin/nope", 0, argv, envp).err == 2);
}

int main(int argc, char **argv) {
    if (argc > 1 && eq(argv[1], "child", 6)) {
        poly_write(9, "child hi\n", 9);
        return 7;
    }
    files();
    directory();
    memory();
    system_info();
    signals();
    child_process(argv[0]);
    say(failures == 0 ? "bsd: done\n" : "bsd: failed\n");
    return failures;
}
