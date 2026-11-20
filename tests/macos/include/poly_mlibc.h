/*
 * Support for macOS test programs about libSystem's own functions (ADR 0182,
 * the libSystem batch). A test that includes this header says so in its header
 * comment with the line `poly-mac: libsystem` (`mac build` then lets it
 * import libSystem symbols; all other tests import nothing, see
 * poly_mtest.h), includes the system headers it needs, and defines `int
 * test_main(void)`. It is linked against libSystem only.
 *
 * What is reported does not go through the code under test: output is one
 * raw `write` system call per piece to fd 1, never stdio, so a library that
 * breaks `snprintf` or `malloc` cannot hide that in its own output.
 *
 * Output protocol (as poly_mtest.h and the Linux corpus):
 *
 *     ok <check>
 *     not ok <check> (got <value>)
 *     value <name> <value>
 *     summary <passed> passed, <failed> failed
 *
 * The exit status is 0 when every check passed, 1 otherwise; the program
 * returns from `main`, so exit handlers (`atexit`) run, and their output
 * (through `put`) follows the summary. Never print addresses, thread ids,
 * times or anything else that differs between runs: every test runs several
 * times per architecture and must print the same bytes. A value may differ
 * between x86_64 and arm64; each is recorded on its own. A test starts in a
 * fresh, empty scratch directory with an empty environment.
 *
 * Written for Poly from public sources and from observation only: the
 * system call conventions are those found by experiment on the oracle Mac
 * (see poly_mtest.h). No Apple or third-party source was read.
 */
#ifndef POLY_MLIBC_H
#define POLY_MLIBC_H

#include <stddef.h>
#include <stdint.h>

#pragma clang diagnostic ignored "-Wdeprecated-declarations"
#pragma clang diagnostic ignored "-Wunused-function"
#pragma clang diagnostic ignored "-Wnonnull"
#pragma clang diagnostic ignored "-Wnon-power-of-two-alignment"

#if defined(__x86_64__)
#define POLY_ARCH "x86_64"
static inline long poly_raw_write(int fd, const void *buf, unsigned long n) {
    long rax = 0x2000004;
    __asm__ volatile("syscall" : "+a"(rax), "+D"(fd), "+S"(buf), "+d"(n) : : "rcx", "r11", "memory", "cc");
    return rax;
}
#elif defined(__arm64__) || defined(__aarch64__)
#define POLY_ARCH "arm64"
static inline long poly_raw_write(int fd, const void *buf, unsigned long n) {
    register long x0 __asm__("x0") = fd;
    register const void *x1 __asm__("x1") = buf;
    register unsigned long x2 __asm__("x2") = n;
    register long x16 __asm__("x16") = 4;
    __asm__ volatile("svc #0x80" : "+r"(x0) : "r"(x1), "r"(x2), "r"(x16) : "memory", "cc");
    return x0;
}
#else
#error "unsupported architecture"
#endif

static inline unsigned long poly_len(const char *s) {
    unsigned long n = 0;
    while (s[n]) n++;
    return n;
}
static inline void put(const char *s) { poly_raw_write(1, s, poly_len(s)); }
static inline void put_n(const char *s, unsigned long n) { poly_raw_write(1, s, n); }
static inline void put_num(long long v) {
    char buf[24];
    int i = sizeof buf;
    unsigned long long u = v < 0 ? -(unsigned long long)v : (unsigned long long)v;
    do buf[--i] = (char)('0' + u % 10);
    while (u /= 10);
    if (v < 0) buf[--i] = '-';
    poly_raw_write(1, buf + i, sizeof buf - i);
}
/* A string with its non-printable bytes written as \xNN, so that it can be
 * recorded and compared as text. */
static inline void put_esc(const char *s, unsigned long n) {
    static const char hex[] = "0123456789abcdef";
    for (unsigned long i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        if (c >= 0x20 && c < 0x7f && c != '\\') {
            poly_raw_write(1, s + i, 1);
        } else {
            char e[4] = {'\\', 'x', hex[c >> 4], hex[c & 15]};
            poly_raw_write(1, e, 4);
        }
    }
}

static int poly_passed, poly_failed;

/* Report one check; `got` is printed when it fails. */
static inline void check(const char *name, int ok, long long got) {
    if (ok) {
        poly_passed++;
        put("ok ");
        put(name);
        put("\n");
    } else {
        poly_failed++;
        put("not ok ");
        put(name);
        put(" (got ");
        put_num(got);
        put(")\n");
    }
}
/* A check on a string: ok when it equals `want`; shows it escaped when not. */
static inline void check_str(const char *name, const char *got, const char *want) {
    int ok = got != NULL;
    if (ok) {
        unsigned long a = poly_len(got), b = poly_len(want);
        ok = a == b;
        for (unsigned long i = 0; ok && i < a; i++) ok = got[i] == want[i];
    }
    if (ok) {
        check(name, 1, 0);
    } else {
        poly_failed++;
        put("not ok ");
        put(name);
        put(" (got ");
        if (got) {
            put("\"");
            put_esc(got, poly_len(got));
            put("\"");
        } else {
            put("NULL");
        }
        put(")\n");
    }
}
/* A deterministic value worth recording (it may differ per architecture). */
static inline void value(const char *name, long long v) {
    put("value ");
    put(name);
    put(" ");
    put_num(v);
    put("\n");
}
static inline void value_str(const char *name, const char *s) {
    put("value ");
    put(name);
    put(" \"");
    put_esc(s, poly_len(s));
    put("\"\n");
}

static inline int finish(void) {
    put("summary ");
    put_num(poly_passed);
    put(" passed, ");
    put_num(poly_failed);
    put(" failed\n");
    return poly_failed ? 1 : 0;
}

/* What the program was started with. A test that needs a child runs itself
 * again (`posix_spawn` of poly_argv[0], the absolute path the runner gave)
 * with an argument that selects its child part; `is_child` tells which. */
static int poly_argc;
static char **poly_argv;
static char **poly_envp;
static inline int is_child(const char *mode) {
    if (poly_argc < 2) return 0;
    const char *a = poly_argv[1];
    while (*a && *a == *mode) a++, mode++;
    return *a == *mode;
}

int test_main(void);

int main(int argc, char **argv, char **envp) {
    poly_argc = argc;
    poly_argv = argv;
    poly_envp = envp;
    return test_main();
}

#endif
