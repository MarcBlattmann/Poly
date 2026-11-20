/*
 * Support for macOS test programs about CoreFoundation's own functions (ADR
 * 0182, the CoreFoundation batch). A test that includes this header says so in
 * its header comment with the line `poly-mac: corefoundation` (`mac build` then
 * links it with `-framework CoreFoundation` and lets it import CoreFoundation's
 * own symbols; all other tests import nothing, see poly_mtest.h), and defines
 * `int test_main(void)`. The CoreFoundation types and constants come from the
 * public SDK headers (<CoreFoundation/CoreFoundation.h>).
 *
 * What is reported does not go through the code under test: output is the raw
 * `write` system call of poly_mtest.h (`check`, `value`, `finish`), never
 * stdio. The helpers below build and read strings with CoreFoundation itself;
 * a test about one of those functions says so and checks it by other means
 * (bytes, characters).
 *
 * Output protocol (as poly_mtest.h and the Linux corpus):
 *
 *     ok <check>
 *     not ok <check> (got <value>)
 *     value <name> <value>
 *     summary <passed> passed, <failed> failed
 *
 * Never print addresses, thread ids, times or anything else that differs
 * between runs: every test runs several times per architecture and must print
 * the same bytes. A value may differ between x86_64 and arm64; each is recorded
 * on its own. A test starts in a fresh, empty scratch directory with an empty
 * environment.
 *
 * Written for Poly from public sources and from observation only. No Apple or
 * third-party source was read; no code is copied from anywhere.
 */
#ifndef POLY_MCF_H
#define POLY_MCF_H

#include <CoreFoundation/CoreFoundation.h>

/* Raw BSD system calls for what a test needs beside CoreFoundation (output,
 * exit, files, processes): the conventions of poly_mtest.h. The result is the
 * errno (> 0) when the call failed, else a value >= 0 in `ret`. */
typedef long i64;
typedef unsigned long u64;
typedef struct {
    i64 ret;
    i64 err;
} sysret;

#define SYS_exit 1
#define SYS_read 3
#define SYS_write 4
#define SYS_open 5
#define SYS_close 6
#define SYS_unlink 10
#define SYS_execve 59
#define SYS_mkdir 136
#define SYS_rmdir 137
#define SYS_chmod 15
#define SYS_fork 2
#define SYS_wait4 7

#if defined(__x86_64__)
static inline sysret poly_bsd3(i64 n, i64 a, i64 b, i64 c) {
    i64 rax = 0x2000000 | n, rdx = c;
    unsigned char carry;
    __asm__ volatile("syscall\n\tsetc %1" : "+a"(rax), "=q"(carry), "+d"(rdx) : "D"(a), "S"(b) : "rcx", "r11", "memory", "cc");
    sysret r = {rax, 0};
    if (carry) r.err = rax;
    return r;
}
#elif defined(__arm64__) || defined(__aarch64__)
static inline sysret poly_bsd3(i64 n, i64 a, i64 b, i64 c) {
    register i64 x0 __asm__("x0") = a;
    register i64 x1 __asm__("x1") = b;
    register i64 x2 __asm__("x2") = c;
    register i64 x16 __asm__("x16") = n;
    i64 carry;
    __asm__ volatile("svc #0x80\n\tcset %[c], cs"
                     : "+r"(x0), "+r"(x1), "+r"(x2), "+r"(x16), [c] "=r"(carry)
                     :
                     : "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x17", "memory", "cc");
    sysret r = {x0, 0};
    if (carry) r.err = x0;
    return r;
}
#else
#error "macOS tests are built for x86_64 and arm64"
#endif
#define bsd3(n, a, b, c) poly_bsd3((n), (i64)(a), (i64)(b), (i64)(c))

/* The compiler may emit calls to these; defined here so that a test imports
 * nothing from libSystem. */
#undef memset
#undef memcpy
void *memset(void *dst, int c, size_t n) {
    unsigned char *d = dst;
    while (n--) *d++ = (unsigned char)c;
    return dst;
}
void *memcpy(void *dst, const void *src, size_t n) {
    unsigned char *d = dst;
    const unsigned char *s = src;
    while (n--) *d++ = *s++;
    return dst;
}

static __attribute__((unused)) size_t str_len(const char *s) {
    size_t n = 0;
    while (s[n]) n++;
    return n;
}
static __attribute__((unused)) int str_eq(const char *a, const char *b) {
    while (*a && *a == *b) a++, b++;
    return *a == *b;
}
static __attribute__((unused)) int mem_eq(const void *a, const void *b, size_t n) {
    const unsigned char *x = a, *y = b;
    for (size_t i = 0; i < n; i++)
        if (x[i] != y[i]) return 0;
    return 1;
}
static __attribute__((unused)) void put(const char *s) { bsd3(SYS_write, 1, s, str_len(s)); }
static __attribute__((unused)) void put_num(i64 v) {
    char buf[24];
    int i = sizeof buf;
    u64 u = v < 0 ? -(u64)v : (u64)v;
    do buf[--i] = (char)('0' + u % 10);
    while (u /= 10);
    if (v < 0) buf[--i] = '-';
    bsd3(SYS_write, 1, buf + i, sizeof buf - i);
}

static int passed, failed;

/* Report one check; `got` is printed when it fails. */
static __attribute__((unused)) void check(const char *name, int ok, i64 got) {
    if (ok) {
        passed++;
        put("ok ");
        put(name);
        put("\n");
    } else {
        failed++;
        put("not ok ");
        put(name);
        put(" (got ");
        put_num(got);
        put(")\n");
    }
}

/* A deterministic value worth recording (it may differ per architecture). */
static __attribute__((unused)) void value(const char *name, i64 v) {
    put("value ");
    put(name);
    put(" ");
    put_num(v);
    put("\n");
}
static __attribute__((unused)) void value_str(const char *name, const char *s) {
    put("value ");
    put(name);
    put(" ");
    put(s);
    put("\n");
}

static __attribute__((unused)) int finish(void) {
    put("summary ");
    put_num(passed);
    put(" passed, ");
    put_num(failed);
    put(" failed\n");
    return failed ? 1 : 0;
}

/* What the program was started with: a test that starts a child runs itself
 * again (execve of poly_argv[0]) with an argument that selects its child part. */
static int poly_argc;
static char **poly_argv;
static __attribute__((unused)) int is_child(const char *mode) { return poly_argc > 1 && str_eq(poly_argv[1], mode); }

static __attribute__((noreturn, unused)) void poly_exit(int code) {
    for (;;) bsd3(SYS_exit, code, 0, 0);
}

/* A CFString of the UTF-8 bytes `s` (the caller never releases it: the
 * process is short-lived). */
static __attribute__((unused)) CFStringRef S(const char *s) {
    return CFStringCreateWithCString(kCFAllocatorDefault, s, kCFStringEncodingUTF8);
}

/* Whether `s` holds exactly the UTF-8 text `c`, compared through
 * CFStringGetCString. */
static __attribute__((unused)) int cf_is(CFTypeRef o, const char *c) {
    char buf[1024];
    CFStringRef s = (CFStringRef)o;
    if (!s || CFGetTypeID(o) != CFStringGetTypeID()) return 0;
    if (!CFStringGetCString(s, buf, sizeof buf, kCFStringEncodingUTF8)) return 0;
    return str_eq(buf, c);
}

/* Whether `s` holds exactly the `n` UTF-16 code units `want`, read with
 * CFStringGetLength and CFStringGetCharacterAtIndex. */
static __attribute__((unused)) int cf_units(CFStringRef s, const UniChar *want, CFIndex n) {
    if (!s || CFStringGetLength(s) != n) return 0;
    for (CFIndex i = 0; i < n; i++)
        if (CFStringGetCharacterAtIndex(s, i) != want[i]) return 0;
    return 1;
}
#define UNITS(s, ...) cf_units((s), (const UniChar[]){__VA_ARGS__}, (CFIndex)(sizeof((const UniChar[]){__VA_ARGS__}) / sizeof(UniChar)))

/* Print `name` with the string's UTF-8 text (or `<null>`). */
static __attribute__((unused)) void value_cf(const char *name, CFStringRef s) {
    char buf[1024];
    if (!s) return value_str(name, "<null>");
    if (!CFStringGetCString(s, buf, sizeof buf, kCFStringEncodingUTF8)) return value_str(name, "<not utf-8>");
    value_str(name, buf);
}


/* `CK(cond)` reports a check named by its own text; `CKV(cond, got)` also
 * shows a number when it fails. */
#define CK(...) check(#__VA_ARGS__, (__VA_ARGS__) ? 1 : 0, 0)
#define CKV(c, got) check(#c, (c) ? 1 : 0, (i64)(got))

/* A CFString read back as UTF-8 into `buf` (empty when it is NULL or cannot be
 * converted). */
static __attribute__((unused)) const char *cf_c(CFStringRef s, char *buf, CFIndex n) {
    buf[0] = 0;
    if (s) CFStringGetCString(s, buf, n, kCFStringEncodingUTF8);
    return buf;
}

/* Print `name` with the text of `s`, the hexadecimal addresses in a
 * description (`<CFData 0x7f.. [0x1f..]>`) replaced by `0xADDR`: a pointer is
 * the one thing that differs from run to run. A hexadecimal number after
 * `bytes = ` stays. */
static __attribute__((unused)) void value_masked(const char *name, CFStringRef s) {
    char in[1024], out[1024];
    int j = 0;
    if (!s) return value_str(name, "<null>");
    cf_c(s, in, sizeof in);
    for (int i = 0; in[i] && j < (int)sizeof out - 8; i++) {
        int addr = 0;
        if (in[i] == '0' && in[i + 1] == 'x' && i > 0) {
            if (in[i - 1] == '[') addr = 1;
            else if (in[i - 1] == ' ') {
                int k = i - 2;
                while (k >= 0 && in[k] != ' ' && in[k] != '<' && in[k] != '=') k--;
                if (k >= 0 && in[k] == '<') addr = 1;
            }
        }
        if (addr) {
            for (const char *t = "0xADDR"; *t; t++) out[j++] = *t;
            i += 2;
            while ((in[i] >= '0' && in[i] <= '9') || (in[i] >= 'a' && in[i] <= 'f')) i++;
            i--;
        } else {
            out[j++] = in[i];
        }
    }
    out[j] = 0;
    value_str(name, out);
}

/* An allocator of CoreFoundation's own kind whose calls are counted: memory
 * comes from a static arena and is never reused, so a pointer stays unique.
 * `ca_allocs` counts allocate calls (every object a test makes through it is
 * one allocation), `ca_deallocs` its deallocate calls. */
static char ca_arena[1 << 18];
static unsigned long ca_used;
static int ca_allocs, ca_deallocs;
static void *ca_allocate(CFIndex n, CFOptionFlags hint, void *info) {
    (void)hint;
    (void)info;
    ca_used = (ca_used + 15) & ~15UL;
    void *p = ca_arena + ca_used;
    ca_used += (unsigned long)n;
    ca_allocs++;
    return p;
}
static void *ca_reallocate(void *p, CFIndex n, CFOptionFlags hint, void *info) {
    void *q = ca_allocate(n, hint, info);
    if (p) memcpy(q, p, (size_t)n);
    return q;
}
static void ca_deallocate(void *p, void *info) {
    (void)p;
    (void)info;
    ca_deallocs++;
}
static __attribute__((unused)) CFAllocatorRef counting_allocator(void) {
    CFAllocatorContext c = {0, 0, 0, 0, 0, ca_allocate, ca_reallocate, ca_deallocate, 0};
    return CFAllocatorCreate(kCFAllocatorDefault, &c);
}


/* --- A small application bundle built at run time --------------------------
 * A test about CFBundle builds `T.app` in its scratch directory (a directory
 * tree with an Info.plist, a few resources and a copy of its own executable),
 * then starts that copy with execve: the process that continues is the
 * executable of a bundle, so the main bundle is that application. The copy
 * recognises itself by the argument `--in-app`. Everything here is raw system
 * calls. Call `cf_enter_app(info_plist_text)` first thing in `test_main`: in the
 * first process it never returns, in the copy it returns 1. */
#define O_WRONLY_ 1
#define O_CREAT_ 0x200
#define O_TRUNC_ 0x400
#define SYS_chmod_ 15
#define SYS_mkdir_ 136

static __attribute__((unused)) int cf_mkdir(const char *path) { return bsd3(SYS_mkdir_, path, 0755, 0).err == 0; }

static __attribute__((unused)) int cf_write_file(const char *path, const char *data, size_t n) {
    sysret fd = bsd3(SYS_open, path, O_WRONLY_ | O_CREAT_ | O_TRUNC_, 0644);
    if (fd.err) return 0;
    sysret w = bsd3(SYS_write, fd.ret, data, n);
    bsd3(SYS_close, fd.ret, 0, 0);
    return w.err == 0 && (size_t)w.ret == n;
}

static __attribute__((unused)) int cf_copy_file(const char *from, const char *to) {
    static char buf[1 << 16];
    sysret in = bsd3(SYS_open, from, 0, 0);
    if (in.err) return 0;
    sysret out = bsd3(SYS_open, to, O_WRONLY_ | O_CREAT_ | O_TRUNC_, 0755);
    if (out.err) return 0;
    for (;;) {
        sysret r = bsd3(SYS_read, in.ret, buf, sizeof buf);
        if (r.err) return 0;
        if (r.ret == 0) break;
        for (i64 off = 0; off < r.ret;) {
            sysret w = bsd3(SYS_write, out.ret, buf + off, r.ret - off);
            if (w.err) return 0;
            off += w.ret;
        }
    }
    bsd3(SYS_close, in.ret, 0, 0);
    bsd3(SYS_close, out.ret, 0, 0);
    bsd3(SYS_chmod_, to, 0755, 0);
    return 1;
}

/* Build the bundle: `T.app/Contents/{Info.plist, MacOS/t, Resources/...}`. The
 * resources: `hello.txt` ("hi\n"), `sub/deep.txt`, `en.lproj/loc.txt`. */
static __attribute__((unused)) int cf_build_app(const char *plist) {
    return cf_mkdir("T.app") && cf_mkdir("T.app/Contents") && cf_mkdir("T.app/Contents/MacOS") &&
           cf_mkdir("T.app/Contents/Resources") && cf_mkdir("T.app/Contents/Resources/sub") &&
           cf_mkdir("T.app/Contents/Resources/en.lproj") &&
           cf_write_file("T.app/Contents/Info.plist", plist, str_len(plist)) &&
           cf_write_file("T.app/Contents/Resources/hello.txt", "hi\n", 3) &&
           cf_write_file("T.app/Contents/Resources/sub/deep.txt", "deep\n", 5) &&
           cf_write_file("T.app/Contents/Resources/en.lproj/loc.txt", "loc\n", 4) &&
           cf_copy_file(poly_argv[0], "T.app/Contents/MacOS/t");
}

/* As cf_enter_app, but the copy gets a second argument `extra` (if not NULL) and
 * the environment `envp` (NULL-terminated, NULL for an empty one). */
static __attribute__((unused)) int cf_enter_app_ex(const char *plist, const char *extra, char **envp) {
    if (is_child("--in-app")) return 1;
    if (!cf_build_app(plist)) {
        check("the test application could be built", 0, 0);
        finish();
        poly_exit(1);
    }
    char *argv[4] = {(char *)"T.app/Contents/MacOS/t", (char *)"--in-app", (char *)extra, 0};
    char *noenv[1] = {0};
    sysret r = bsd3(SYS_execve, "T.app/Contents/MacOS/t", argv, envp ? envp : noenv);
    check("the test application could be started", 0, r.err);
    finish();
    poly_exit(1);
}

static __attribute__((unused)) int cf_enter_app(const char *plist) {
    if (is_child("--in-app")) return 1;
    if (!cf_build_app(plist)) {
        check("the test application could be built", 0, 0);
        finish();
        poly_exit(1);
    }
    char *argv[3] = {(char *)"T.app/Contents/MacOS/t", (char *)"--in-app", 0};
    char *envp[1] = {0};
    sysret r = bsd3(SYS_execve, "T.app/Contents/MacOS/t", argv, envp);
    check("the test application could be started", 0, r.err);
    finish();
    poly_exit(1);
}

/* The standard Info.plist the bundle tests use. */
#define CF_APP_PLIST \
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n" \
    "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n" \
    "<plist version=\"1.0\"><dict>\n" \
    "<key>CFBundleIdentifier</key><string>test.poly.probe</string>\n" \
    "<key>CFBundleName</key><string>Probe</string>\n" \
    "<key>CFBundleExecutable</key><string>t</string>\n" \
    "<key>CFBundleVersion</key><string>42</string>\n" \
    "<key>CFBundleShortVersionString</key><string>1.2.3</string>\n" \
    "<key>CFBundlePackageType</key><string>APPL</string>\n" \
    "<key>MyInt</key><integer>7</integer><key>MyBool</key><true/>\n" \
    "<key>MyArr</key><array><string>a</string><string>b</string></array>\n" \
    "</dict></plist>\n"

/* Whether the text of `s` ends with `tail` / contains `part`. */
static __attribute__((unused)) int cf_ends(CFStringRef s, const char *tail) {
    return s && CFStringHasSuffix(s, CFStringCreateWithCString(kCFAllocatorDefault, tail, kCFStringEncodingUTF8));
}
static __attribute__((unused)) int cf_url_path_ends(CFURLRef u, const char *tail) {
    CFStringRef p = u ? CFURLCopyFileSystemPath(u, kCFURLPOSIXPathStyle) : NULL;
    return cf_ends(p, tail);
}

/* Preferences: tests use a domain of their own (`test.poly.cfspec.<name>`),
 * never a real application's. `cf_prefs_clean` removes every key the domain
 * holds for the current user and writes the change out; a test calls it first
 * and last. */
static __attribute__((unused)) void cf_prefs_clean(CFStringRef domain) {
    CFArrayRef keys = CFPreferencesCopyKeyList(domain, kCFPreferencesCurrentUser, kCFPreferencesAnyHost);
    for (CFIndex i = 0; keys && i < CFArrayGetCount(keys); i++)
        CFPreferencesSetAppValue((CFStringRef)CFArrayGetValueAtIndex(keys, i), NULL, domain);
    CFPreferencesAppSynchronize(domain);
}

int test_main(void);

/* poly_mfn.h defines its own main (it wraps the test in an autorelease pool). */
#ifndef POLY_MCF_NO_MAIN
int main(int argc, char **argv) {
    poly_argc = argc;
    poly_argv = argv;
    poly_exit(test_main());
}
#endif

#endif
