/* What the MIG-generated stubs and the exception tests need, on raw
 * traps and calls: Poly's tests link no Apple library (ADR 0238). The
 * numbers are the ones of <mach/mach_traps.h> and <sys/syscall.h>. */
#include "support.h"

NDR_record_t NDR_record = {0, 0, 0, 0, 1, 0, 0, 0};
mach_port_t mach_task_self_;
static mach_port_t reply_port;

long poly_sys(long n, long a, long b, long c, long d, long e, long f) {
    long ret;
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory", "cc");
    return ret;
}

void *memcpy(void *dst, const void *src, size_t n) {
    unsigned char *d = dst;
    const unsigned char *s = src;
    while (n--) *d++ = *s++;
    return dst;
}
void *__memcpy_chk(void *dst, const void *src, size_t n, size_t dn) { (void)dn; return memcpy(dst, src, n); }
void *memset(void *dst, int c, size_t n) {
    unsigned char *d = dst;
    while (n--) *d++ = (unsigned char)c;
    return dst;
}

mach_port_t mig_get_reply_port(void) {
    if (!reply_port) reply_port = (mach_port_t)poly_sys(MACH(26), 0, 0, 0, 0, 0, 0);
    return reply_port;
}
void mig_dealloc_reply_port(mach_port_t p) { (void)p; reply_port = 0; }
void mig_put_reply_port(mach_port_t p) { (void)p; }
void mach_msg_destroy(mach_msg_header_t *m) { (void)m; }
boolean_t voucher_mach_msg_set(mach_msg_header_t *m) { (void)m; return 0; }

kern_return_t mach_port_allocate(ipc_space_t task, mach_port_right_t right, mach_port_name_t *name) {
    return (kern_return_t)poly_sys(MACH(16), task, right, (long)name, 0, 0, 0);
}
kern_return_t mach_port_insert_right(ipc_space_t task, mach_port_name_t name, mach_port_t port, mach_msg_type_name_t t) {
    return (kern_return_t)poly_sys(MACH(21), task, name, port, t, 0, 0);
}

void put(const char *s) {
    size_t n = 0;
    while (s[n]) n++;
    poly_sys(BSD(4), 1, (long)s, (long)n, 0, 0, 0);
}
void put_num(long v) {
    char b[24];
    int i = 23;
    int neg = v < 0;
    unsigned long u = neg ? (unsigned long)-v : (unsigned long)v;
    b[i] = 0;
    do { b[--i] = (char)('0' + u % 10); u /= 10; } while (u);
    if (neg) b[--i] = '-';
    put(b + i);
}
static int passed, failed;
void check(const char *name, int ok, long got) {
    if (ok) { passed++; put("ok "); put(name); put("\n"); return; }
    failed++;
    put("not ok "); put(name); put(" (got "); put_num(got); put(")\n");
}
void value(const char *name, long v) { put("value "); put(name); put(" "); put_num(v); put("\n"); }
int finish(void) {
    put("summary "); put_num(passed); put(" passed, "); put_num(failed); put(" failed\n");
    return failed ? 1 : 0;
}
__attribute__((noreturn)) void poly_exit(int code) {
    for (;;) poly_sys(BSD(1), code, 0, 0, 0, 0, 0);
}

/* posix_spawn(pid, path, attributes (none), argv, envp) and wait4. */
int spawn(const char *path, char *const argv[]) {
    int pid = 0;
    static char *const envp[] = {0};
    long r = poly_sys(BSD(244), (long)&pid, (long)path, 0, (long)argv, (long)envp, 0);
    return r ? -1 : pid;
}
int wait_child(int pid) {
    int status = 0;
    poly_sys(BSD(7), pid, (long)&status, 0, 0, 0, 0);
    return status;
}
