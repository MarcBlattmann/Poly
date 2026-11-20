/*
 * Support for macOS tests of libSystem functions (the bootstrap server calls,
 * mach_port_*, XPC, posix_spawn attributes). A test that includes this file
 * says `poly-mac: libsystem` in its header comment, so that `mac build`
 * lets it import libSystem's symbols (tools/pipeline/src/mac.rs); it loads
 * nothing else. Written for Poly from public documentation and observation
 * only; no Apple or third-party source was read.
 *
 * Output protocol as poly_mtest.h (ok / not ok / value / summary), written
 * with write(2) so that parent and child lines come out in the order they
 * ran. Never print names, ports, pids or times: every test runs several
 * times and must print the same bytes.
 *
 * A test defines `int test_main(void)`. A test that starts a child runs
 * itself again with an argument (start_self) and handles that in
 * `if (is_child("mode"))` at the top of test_main; the child's lines are
 * printed by the child and its exit status is the parent's to check.
 */
#ifndef POLY_MAPI_H
#define POLY_MAPI_H

#include <mach/mach.h>
#include <mach/mach_error.h>
#include <mach/notify.h>
#include <servers/bootstrap.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include <dispatch/dispatch.h>
#include <xpc/xpc.h>

#pragma clang diagnostic ignored "-Wdeprecated-declarations"

extern char **environ;

static int poly_argc;
static char **poly_argv;
static int passed, failed;

static __attribute__((unused)) void put(const char *s) { (void)!write(1, s, strlen(s)); }
static __attribute__((unused)) void put_num(long v) {
    char buf[32];
    snprintf(buf, sizeof buf, "%ld", v);
    put(buf);
}

/* Report one check; `got` is printed when it fails. */
static __attribute__((unused)) void check(const char *name, int ok, long got) {
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
/* A deterministic value worth recording. */
static __attribute__((unused)) void value(const char *name, long v) {
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
    put(s ? s : "(null)");
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

/* The type bits of a name in this task, or 0xdead0000 | the kern_return_t. */
static __attribute__((unused)) unsigned type_of(mach_port_t name) {
    mach_port_type_t t = 0;
    kern_return_t kr = mach_port_type(mach_task_self(), name, &t);
    return kr ? 0xdead0000u | (unsigned)kr : (unsigned)t;
}
/* Whether a name is invalid in this task now. */
static __attribute__((unused)) int is_gone(mach_port_t name) { return type_of(name) == (0xdead0000u | KERN_INVALID_NAME); }
static __attribute__((unused)) unsigned urefs_of(mach_port_t name, mach_port_right_t right) {
    mach_port_urefs_t u = 0xfffff;
    mach_port_get_refs(mach_task_self(), name, right, &u);
    return (unsigned)u;
}
static __attribute__((unused)) mach_port_t new_receive(void) {
    mach_port_t p = MACH_PORT_NULL;
    mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &p);
    return p;
}
/* A receive right that also has a send right, under the same name. */
static __attribute__((unused)) mach_port_t new_port_with_send(void) {
    mach_port_t p = new_receive();
    mach_port_insert_right(mach_task_self(), p, p, MACH_MSG_TYPE_MAKE_SEND);
    return p;
}
/* Destroy a receive right (and with it every right to the port in this task). */
static __attribute__((unused)) void destroy_port(mach_port_t p) { mach_port_destroy(mach_task_self(), p); }

/* A message with only a header: sent to `dst` with a send right copied, id `id`. */
static __attribute__((unused)) kern_return_t send_empty(mach_port_t dst, mach_msg_id_t id, mach_msg_timeout_t timeout_ms) {
    mach_msg_header_t m;
    memset(&m, 0, sizeof m);
    m.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    m.msgh_size = sizeof m;
    m.msgh_remote_port = dst;
    m.msgh_id = id;
    return mach_msg(&m, MACH_SEND_MSG | MACH_SEND_TIMEOUT, sizeof m, 0, MACH_PORT_NULL, timeout_ms, MACH_PORT_NULL);
}
typedef struct {
    mach_msg_header_t h;
    unsigned char body[64];
    mach_msg_trailer_t trailer_room[2];
} poly_rcv_t;
/* Receive one message from `port`, waiting up to `timeout_ms`; returns the
 * kern_return_t, the message in `*m` (its size in m->h.msgh_size). */
static __attribute__((unused)) kern_return_t recv_msg(mach_port_t port, mach_msg_timeout_t timeout_ms, poly_rcv_t *m) {
    memset(m, 0, sizeof *m);
    return mach_msg(&m->h, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof *m, port, timeout_ms, MACH_PORT_NULL);
}
/* How many messages `port` yields within `timeout_ms` of silence; the last
 * one is left in `*last`. */
static __attribute__((unused)) int drain(mach_port_t port, mach_msg_timeout_t timeout_ms, poly_rcv_t *last) {
    int n = 0;
    poly_rcv_t m;
    while (recv_msg(port, timeout_ms, &m) == KERN_SUCCESS) {
        n++;
        if (last) *last = m;
    }
    return n;
}

/* Children: start_self runs this program again with argv[1] = mode and
 * argv[2] = arg (either may be NULL) and returns the child's pid (-1 on
 * failure). */
static __attribute__((unused)) int is_child(const char *mode) { return poly_argc > 1 && strcmp(poly_argv[1], mode) == 0; }
static __attribute__((unused)) char *child_arg(void) { return poly_argc > 2 ? poly_argv[2] : (char *)""; }
static __attribute__((unused)) pid_t start_self(const char *mode, const char *arg) {
    char *argv[4] = {poly_argv[0], (char *)mode, (char *)arg, NULL};
    pid_t pid = -1;
    return posix_spawn(&pid, poly_argv[0], NULL, NULL, argv, environ) ? -1 : pid;
}
/* The exit status of a child, or 1000 + the signal that ended it, or -1. */
static __attribute__((unused)) int wait_child(pid_t pid) {
    int st = 0;
    if (waitpid(pid, &st, 0) != pid) return -1;
    return WIFEXITED(st) ? WEXITSTATUS(st) : 1000 + WTERMSIG(st);
}
/* bootstrap_look_up, repeated for up to `seconds` until it gives `want` (or, when want is
 * KERN_SUCCESS, until it succeeds): the server learns of a process's death a moment after it. */
static __attribute__((unused)) kern_return_t look_up_until(const char *name, kern_return_t want, int seconds, mach_port_t *out) {
    kern_return_t kr = 0;
    for (int i = 0; i < seconds * 20; i++) {
        kr = bootstrap_look_up(bootstrap_port, (char *)name, out);
        if (kr == want) return kr;
        usleep(50000);
    }
    return kr;
}
/* XPC: the description string of an error object, or "-" for anything else. */
static __attribute__((unused)) const char *xpc_error_text(xpc_object_t o) {
    const char *s = (o && xpc_get_type(o) == XPC_TYPE_ERROR) ? xpc_dictionary_get_string(o, XPC_ERROR_KEY_DESCRIPTION) : NULL;
    return s ? s : "-";
}
/* XPC: wait for a semaphore for up to `seconds`; 1 when it was signalled. */
static __attribute__((unused)) int xpc_wait(dispatch_semaphore_t sem, int seconds) {
    return dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)seconds * NSEC_PER_SEC)) == 0;
}
/* A service name nobody else uses: com.poly.spec.<tag>.<pid>. */
static __attribute__((unused)) void service_name(char *buf, size_t n, const char *tag) {
    snprintf(buf, n, "com.poly.spec.%s.%d", tag, (int)getpid());
}

int test_main(void);

int main(int argc, char **argv) {
    poly_argc = argc;
    poly_argv = argv;
    int rc = test_main();
    exit(rc);
}

#endif
