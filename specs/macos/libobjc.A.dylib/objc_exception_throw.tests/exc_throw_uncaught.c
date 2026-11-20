// poly-mac: libobjc
#include "poly_mobjc.h"
@interface Boom : NSObject
@end
@implementation Boom
@end
static volatile long zero;
static const char prefix[] = "libc++abi: terminating due to uncaught exception of type ";
static int start_child(const char *mode, char *buf, int cap, int *status) {
    sysret p = bsd0(SYS_pipe);
    char fdtext[8]; int r = (int)p.ret, w = (int)p.ret2;
    fdtext[0] = '0' + w; fdtext[1] = 0;
    i64 pid = poly_start_self(mode, fdtext, 0);
    bsd1(SYS_close, w);
    int n = 0;
    for (;;) { sysret rd = bsd3(SYS_read, r, buf + n, cap - 1 - n); if (rd.err || rd.ret <= 0) break; n += (int)rd.ret; }
    buf[n] = 0;
    bsd1(SYS_close, r);
    *status = poly_wait(pid);
    return n;
}
int test_main(void) {
    if (is_child("object") || is_child("nil") || is_child("class")) {
        bsd2(SYS_dup2, poly_atoi(poly_argv[2]), 2);
        put("child: before the throw\n");
        if (is_child("object")) objc_exception_throw(objc_alloc_init(objc_getClass("Boom")));
        if (is_child("class")) objc_exception_throw((id)objc_getClass("Boom"));
        objc_exception_throw((id)zero);
        put("child: after the throw\n");
        return 0;
    }
    const char *modes[3] = {"object", "class", "nil"};
    for (int i = 0; i < 3; i++) {
        char buf[512]; int st;
        int n = start_child(modes[i], buf, sizeof buf, &st);
        put("-- "); put(modes[i]); put("\n");
        check("the child did not exit normally", !W_EXITED(st), st);
        value("terminating signal (SIGABRT is 6)", W_TERMSIG(st));
        int pre = n >= (int)sizeof prefix - 1 && mem_eq(buf, prefix, sizeof prefix - 1);
        check("stderr starts with the libc++abi message", pre, 0);
        if (pre) { char *t = buf + sizeof prefix - 1; for (char *q = t; *q; q++) if (*q == '\n') *q = 0; value_str("type named in the message", t); }
    }
    return finish();
}
