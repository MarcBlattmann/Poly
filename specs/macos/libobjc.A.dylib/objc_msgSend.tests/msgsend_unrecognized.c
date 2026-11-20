// poly-mac: libobjc
// objc_msgSend with a selector nobody implements, and no resolver adding it:
// without Foundation no message-forwarding handler is installed, the runtime
// reports "unrecognized selector" on stderr and the process is killed by
// SIGABRT. -forwardingTargetForSelector: and -forwardInvocation: are not
// called. Each case runs in a child process.
#include "poly_mobjc.h"

static int resolver_ran;

@interface F : NSObject
@end
@implementation F
+ (BOOL)resolveInstanceMethod:(SEL)s {
    resolver_ran++;
    put("child: resolver asked for ");
    put(sel_getName(s));
    put("\n");
    return str_eq(sel_getName(s), "claimsToResolve");
}
+ (BOOL)resolveClassMethod:(SEL)s {
    put("child: class resolver asked for ");
    put(sel_getName(s));
    put("\n");
    return NO;
}
- (id)forwardingTargetForSelector:(SEL)s { (void)s; put("child: forwardingTarget called\n"); return 0; }
- (void)forwardInvocation:(id)inv { (void)inv; put("child: forwardInvocation called\n"); }
- (id)methodSignatureForSelector:(SEL)s { (void)s; put("child: methodSignature called\n"); return 0; }
@end

static void child_part(const char *mode) {
    /* stderr carries the runtime's message with an address and a pid. */
    sysret nul = bsd3(SYS_open, "/dev/null", O_WRONLY, 0);
    bsd2(SYS_dup2, nul.ret, 2);
    id f = [F new];
    put("child: start\n");
    if (str_eq(mode, "instance")) ((void (*)(id, SEL))objc_msgSend)(f, sel_registerName("nosuchmethod"));
    if (str_eq(mode, "class")) ((void (*)(id, SEL))objc_msgSend)((id)objc_getClass("F"), sel_registerName("nosuchclassmethod"));
    if (str_eq(mode, "liar")) ((void (*)(id, SEL))objc_msgSend)(f, sel_registerName("claimsToResolve"));
    put("child: survived\n");
    poly_exit(0);
}

static void run(const char *mode) {
    i64 pid = poly_start_self(mode, 0, 0);
    int st = poly_wait(pid);
    put(mode);
    put(": killed by signal ");
    put_num(W_TERMSIG(st));
    put(", exit status ");
    put_num(W_EXITED(st) ? W_EXITSTATUS(st) : -1);
    put("\n");
    check(mode, !W_EXITED(st) && W_TERMSIG(st) == 6, st);
}

int test_main(void) {
    if (is_child("instance") || is_child("class") || is_child("liar")) child_part(poly_argv[1]);
    run("instance");
    run("class");
    run("liar");
    return finish();
}
