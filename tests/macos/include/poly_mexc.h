/*
 * Support for macOS tests of exception ports and thread state: the
 * task/thread exception port calls, thread_get_state / thread_set_state,
 * the exception messages the kernel sends, and what a reply does. A test
 * that includes this header says `poly-mac: libsystem` in its header comment
 * (see poly_mlibc.h, which this includes: output goes through raw write calls
 * and the same ok / not ok / value / summary protocol), and defines
 * `int test_main(void)`.
 *
 * A test makes its own faults in threads of its own process (a write to a
 * PROT_NONE page, an undefined instruction, a breakpoint, an integer
 * division by zero on x86_64) and catches them with a handler thread of its
 * own, which receives the exception messages on an exception port, looks at
 * them, changes the thread's state when it should, and replies. Only
 * test-owned processes and threads are touched. A test that lets a process
 * die by a fault does so in a child (is_child), which first removes the task's
 * crash-report exception port so that no crash report is written.
 *
 * Written for Poly from public sources (the SDK headers <mach/exc.h>,
 * <mach/exception_types.h>, <mach/thread_status.h> for message layouts and
 * constants) and from observation only. No Apple or third-party source was
 * read.
 */
#ifndef POLY_MEXC_H
#define POLY_MEXC_H

#include "poly_mlibc.h"

#include <mach/mach.h>
#include <mach/exception.h>
#include <mach/mach_vm.h>
#include <mach/thread_status.h>
#include <pthread.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

#if defined(__arm64__)
#define EXC_FLAVOR ARM_THREAD_STATE64
#define EXC_STATE_COUNT ARM_THREAD_STATE64_COUNT
#define EXC_NONE_FLAVOR 5
typedef arm_thread_state64_t exc_ts_t;
#else
#define EXC_FLAVOR x86_THREAD_STATE64
#define EXC_STATE_COUNT x86_THREAD_STATE64_COUNT
#define EXC_NONE_FLAVOR 13
typedef x86_thread_state64_t exc_ts_t;
#endif

static inline unsigned long exc_get_pc(exc_ts_t *s) {
#if defined(__arm64__)
    return (unsigned long)__darwin_arm_thread_state64_get_pc(*s);
#else
    return (unsigned long)s->__rip;
#endif
}
static inline void exc_set_pc(exc_ts_t *s, unsigned long pc) {
#if defined(__arm64__)
    __darwin_arm_thread_state64_set_pc_fptr(*s, (void *)pc);
#else
    s->__rip = pc;
#endif
}
/* The register the register tests use: x9 on arm64, rax on x86_64. */
static inline unsigned long exc_get_reg(exc_ts_t *s) {
#if defined(__arm64__)
    return s->__x[9];
#else
    return s->__rax;
#endif
}
static inline void exc_set_reg(exc_ts_t *s, unsigned long v) {
#if defined(__arm64__)
    s->__x[9] = v;
#else
    s->__rax = v;
#endif
}

static inline unsigned exc_type_of(mach_port_t name) {
    mach_port_type_t t = 0;
    kern_return_t kr = mach_port_type(mach_task_self(), name, &t);
    return kr ? 0xdead0000u | (unsigned)kr : (unsigned)t;
}
static inline mach_port_t exc_new_port(void) {
    mach_port_t p = MACH_PORT_NULL;
    mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &p);
    mach_port_insert_right(mach_task_self(), p, p, MACH_MSG_TYPE_MAKE_SEND);
    return p;
}
static inline unsigned exc_send_urefs(mach_port_t p) {
    mach_port_urefs_t u = 0xfffff;
    mach_port_get_refs(mach_task_self(), p, MACH_PORT_RIGHT_SEND, &u);
    return (unsigned)u;
}

/* What the handler saw of the last exception message. */
typedef struct {
    int id;                 /* msgh_id: 2401..2403 (32-bit codes), 2405..2407 (64-bit codes) */
    unsigned size;          /* msgh_size without the trailer */
    int complex;            /* MACH_MSGH_BITS_COMPLEX */
    int reply_disp;         /* the disposition of the remote (reply) port: 18 = MOVE_SEND_ONCE */
    int has_local;          /* the local port is set (the exception port) */
    int exception;          /* EXC_* */
    unsigned code_cnt;
    long long code[2];
    int thread_is_faulter;  /* the thread descriptor is the faulting thread's port (-1: none) */
    int task_is_self;       /* the task descriptor is this task's port (-1: none) */
    int thread_disp, task_disp;
    int flavor;             /* state variants */
    unsigned state_cnt;
    unsigned trailer_size;
    unsigned long pc;       /* the pc of the state the handler read */
    unsigned long reg;      /* the test register (x9 / rax) of that state */
} exc_seen_t;

/* A handler thread on its own exception port. */
typedef struct {
    mach_port_t port;       /* receive+send */
    pthread_t thread;
    /* what to do */
    volatile int reply_code;        /* the RetCode to reply: KERN_SUCCESS resumes, anything else refuses */
    volatile int pc_advance;        /* added to the pc of a non-access exception before resuming */
    volatile unsigned long fix_addr;/* an access to this page is cured by making it readable and writable */
    volatile int set_reg;           /* set the test register to reg_val in the state it resumes with */
    volatile unsigned long reg_val;
    volatile int xor_state;         /* STATE variants: flip the first word of the state in the reply (not the pc) */
    volatile int ignore_reply;      /* do not reply at all */
    /* what happened */
    volatile int count;
    exc_seen_t seen;
} exc_handler_t;

static mach_port_t exc_faulting_thread;

static void *exc_handler_main(void *arg) {
    exc_handler_t *h = arg;
    for (;;) {
        struct {
            mach_msg_header_t hd;
            unsigned char b[4096];
        } m;
        memset(&m, 0, sizeof m);
        kern_return_t k = mach_msg(&m.hd, MACH_RCV_MSG | MACH_RCV_LARGE, 0, sizeof m, h->port, MACH_MSG_TIMEOUT_NONE, MACH_PORT_NULL);
        if (k) return NULL;
        exc_seen_t s;
        memset(&s, 0, sizeof s);
        int id = m.hd.msgh_id;
        int codes64 = id >= 2405 && id <= 2407;
        int ident = id == 2401 || id == 2403 || id == 2405 || id == 2407;
        int state = id == 2402 || id == 2403 || id == 2406 || id == 2407;
        if (id < 2401 || id > 2407 || id == 2404) continue;
        s.id = id;
        s.size = m.hd.msgh_size;
        s.complex = (m.hd.msgh_bits & MACH_MSGH_BITS_COMPLEX) != 0;
        s.reply_disp = MACH_MSGH_BITS_REMOTE(m.hd.msgh_bits);
        s.has_local = m.hd.msgh_local_port != MACH_PORT_NULL;
        unsigned char *p = m.b;
        mach_port_t thr = MACH_PORT_NULL, tsk = MACH_PORT_NULL;
        if (ident) {
            mach_msg_port_descriptor_t *d = (mach_msg_port_descriptor_t *)(p + 4);
            thr = d[0].name;
            tsk = d[1].name;
            s.thread_disp = d[0].disposition;
            s.task_disp = d[1].disposition;
            p += 4 + 2 * sizeof(mach_msg_port_descriptor_t);
        }
        p += 8; /* NDR record */
        s.exception = *(int *)p;
        p += 4;
        s.code_cnt = *(unsigned *)p;
        p += 4;
        if (codes64) {
            memcpy(s.code, p, 16);
            p += 16;
        } else {
            int c[2];
            memcpy(c, p, 8);
            s.code[0] = c[0];
            s.code[1] = (unsigned)c[1]; /* addresses read as unsigned 32 bits */
            p += 8;
        }
        s.thread_is_faulter = thr ? (thr == exc_faulting_thread) : -1;
        s.task_is_self = tsk ? (tsk == mach_task_self()) : -1;
        exc_ts_t st;
        memset(&st, 0, sizeof st);
        unsigned scnt = 0;
        int flavor = 0;
        if (state) {
            flavor = *(int *)p;
            p += 4;
            scnt = *(unsigned *)p;
            p += 4;
            s.flavor = flavor;
            s.state_cnt = scnt;
            memcpy(&st, p, sizeof st < scnt * 4 ? sizeof st : scnt * 4);
        } else {
            mach_msg_type_number_t c = EXC_STATE_COUNT;
            thread_get_state(thr ? thr : exc_faulting_thread, EXC_FLAVOR, (thread_state_t)&st, &c);
        }
        s.pc = exc_get_pc(&st);
        s.reg = exc_get_reg(&st);
        mach_msg_trailer_t *tr = (mach_msg_trailer_t *)((char *)&m.hd + m.hd.msgh_size);
        s.trailer_size = tr->msgh_trailer_size;
        if (h->reply_code == KERN_SUCCESS) {
            if (s.exception == EXC_BAD_ACCESS) {
                if (h->fix_addr) mach_vm_protect(mach_task_self(), h->fix_addr & ~0xfffUL, 0x1000, 0, VM_PROT_READ | VM_PROT_WRITE);
            } else {
                exc_set_pc(&st, exc_get_pc(&st) + h->pc_advance);
            }
            if (h->set_reg && s.exception == EXC_BREAKPOINT) exc_set_reg(&st, h->reg_val);
            if (!state) thread_set_state(thr ? thr : exc_faulting_thread, EXC_FLAVOR, (thread_state_t)&st, EXC_STATE_COUNT);
        }
        h->seen = s;
        h->count++;
        if (h->ignore_reply) continue;
        struct exc_reply {
            mach_msg_header_t hd;
            NDR_record_t ndr;
            kern_return_t ret;
            int flavor;
            unsigned cnt;
            unsigned char st[sizeof(exc_ts_t) + 16];
        } r;
        memset(&r, 0, sizeof r);
        r.hd.msgh_bits = MACH_MSGH_BITS(MACH_MSGH_BITS_REMOTE(m.hd.msgh_bits), 0);
        r.hd.msgh_remote_port = m.hd.msgh_remote_port;
        r.hd.msgh_id = id + 100;
        r.ndr = NDR_record;
        r.ret = h->reply_code;
        unsigned sz = offsetof(struct exc_reply, flavor);
        if (state) {
            if (h->xor_state && scnt) ((unsigned *)&st)[1] ^= 0xffffffffu;
            r.flavor = flavor;
            r.cnt = scnt;
            memcpy(r.st, &st, sizeof st < scnt * 4 ? sizeof st : scnt * 4);
            sz = offsetof(struct exc_reply, st) + scnt * 4;
        }
        r.hd.msgh_size = sz;
        mach_msg(&r.hd, MACH_SEND_MSG, sz, 0, MACH_PORT_NULL, MACH_MSG_TIMEOUT_NONE, MACH_PORT_NULL);
    }
}

static inline void exc_handler_start(exc_handler_t *h) {
    memset(h, 0, sizeof *h);
    h->port = exc_new_port();
    pthread_create(&h->thread, NULL, exc_handler_main, h);
}

/* The faults. Each runs in a thread of its own (exc_run) so that the
 * handler (another thread) can resume it. exc_fault_* set up what the handler
 * must add to the pc to get past the instruction. */
static exc_handler_t *exc_current;
static void *exc_page;

static void *exc_fault_access(void *a) {
    (void)a;
    exc_faulting_thread = mach_thread_self();
    *(volatile int *)exc_page = 1;
    return NULL;
}
static void *exc_fault_undef(void *a) {
    (void)a;
    exc_faulting_thread = mach_thread_self();
#if defined(__arm64__)
    exc_current->pc_advance = 4;
    __asm__ volatile(".inst 0x00000000");
#else
    exc_current->pc_advance = 2;
    __asm__ volatile("ud2");
#endif
    return NULL;
}
/* A breakpoint with the test register set to 0x42 before it and stored after it: a handler that sets the register
 * shows it in exc_reg_out. */
#if defined(__arm64__)
#define EXC_PC_ADVANCE_BREAK 4
#else
#define EXC_PC_ADVANCE_BREAK 0
#endif
static volatile unsigned long exc_reg_out;
static void *exc_fault_break(void *a) {
    (void)a;
    exc_faulting_thread = mach_thread_self();
#if defined(__arm64__)
    exc_current->pc_advance = 4;
    __asm__ volatile("mov x9, #0x42\n\tbrk #0\n\tmov %0, x9" : "=r"(exc_reg_out)::"x9");
#else
    exc_current->pc_advance = 0;
    __asm__ volatile("mov $0x42, %%eax\n\tint3\n\tmov %%rax, %0" : "=m"(exc_reg_out)::"rax");
#endif
    return NULL;
}
#if defined(__x86_64__)
static void *exc_fault_div(void *a) {
    (void)a;
    exc_faulting_thread = mach_thread_self();
    exc_current->pc_advance = 2;
    __asm__ volatile("xor %%edx, %%edx\n\tmov $100, %%eax\n\txor %%ecx, %%ecx\n\tidiv %%ecx" ::: "rax", "rdx", "rcx");
    return NULL;
}
#endif

/* Prepare the PROT_NONE page the access fault writes to (a high, fixed address, so that its
 * 64-bit address does not fit in 32 bits) and tell the handler to cure it. */
#define EXC_PAGE_ADDR 0x7f1234560000UL
static inline void exc_prepare_page(exc_handler_t *h) {
    exc_page = mmap((void *)EXC_PAGE_ADDR, 4096, PROT_NONE, MAP_ANON | MAP_PRIVATE | MAP_FIXED, -1, 0);
    h->fix_addr = (unsigned long)exc_page;
}
static inline void exc_run(exc_handler_t *h, void *(*fn)(void *)) {
    exc_current = h;
    pthread_t t;
    pthread_create(&t, NULL, fn, NULL);
    pthread_join(t, NULL);
}

/* In a child that is going to die by a fault: no crash report. */
static inline void exc_no_crash_report(void) {
    task_set_exception_ports(mach_task_self(), EXC_MASK_CRASH, MACH_PORT_NULL, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
}

/* Cause the three faults every architecture has, one after the other, each in a thread of its own, with the
 * task's exception ports set for them to `h` with `behavior` (EXCEPTION_*, with MACH_EXCEPTION_CODES or not);
 * the message of each is left in seen[0] (EXC_BAD_ACCESS), seen[1] (EXC_BAD_INSTRUCTION), seen[2] (EXC_BREAKPOINT).
 * Returns the kern_return_t of the last task_set_exception_ports. */
static inline kern_return_t exc_three_faults(exc_handler_t *h, exception_behavior_t behavior, exc_seen_t seen[3]) {
    kern_return_t kr = task_set_exception_ports(mach_task_self(), EXC_MASK_BAD_ACCESS | EXC_MASK_BAD_INSTRUCTION | EXC_MASK_BREAKPOINT, h->port, behavior, EXC_FLAVOR);
    exc_prepare_page(h);
    exc_run(h, exc_fault_access);
    seen[0] = h->seen;
    exc_run(h, exc_fault_undef);
    seen[1] = h->seen;
    exc_run(h, exc_fault_break);
    seen[2] = h->seen;
    return kr;
}
/* The size a message of kernel-delivered exception id `id` has (without trailer) on this architecture. */
static inline unsigned exc_expected_size(int id) {
    int ident = id == 2401 || id == 2403 || id == 2405 || id == 2407;
    int state = id == 2402 || id == 2403 || id == 2406 || id == 2407;
    int codes64 = id >= 2405;
    return 24 + (ident ? 4 + 24 : 0) + 8 + 4 + 4 + (codes64 ? 16 : 8) + (state ? 8 + EXC_STATE_COUNT * 4 : 0);
}

/* Check the three messages of exc_three_faults against what a kernel-delivered exception message of `id_base`
 * (2401, 2402 or 2403) is, with 32-bit codes or (codes64) 64-bit ones: ids, sizes, descriptors, codes. */
static inline void exc_check_layout(int id_base, int codes64, exc_seen_t seen[3], exc_handler_t *h) {
    static const char *names[3] = {"access", "undefined instruction", "breakpoint"};
    static const int excs[3] = {EXC_BAD_ACCESS, EXC_BAD_INSTRUCTION, EXC_BREAKPOINT};
    int id = id_base + (codes64 ? 4 : 0);
    int ident = id_base == 2401 || id_base == 2403;
    int state = id_base == 2402 || id_base == 2403;
    char buf[128];
    check("three exceptions were delivered", h->count == 3, h->count);
    for (int i = 0; i < 3; i++) {
        exc_seen_t *s = &seen[i];
#define EXC_CHECK(what, cond, got) (snprintf(buf, sizeof buf, "%s: %s", names[i], what), check(buf, (cond), (got)))
        EXC_CHECK("message id", s->id == id, s->id);
        EXC_CHECK("message size", s->size == exc_expected_size(id), s->size);
        EXC_CHECK(ident ? "complex, with two port descriptors" : "not complex", s->complex == ident, s->complex);
        EXC_CHECK("the reply right is a send-once right", s->reply_disp == MACH_MSG_TYPE_MOVE_SEND_ONCE, s->reply_disp);
        EXC_CHECK("the local port is set", s->has_local, s->has_local);
        EXC_CHECK("exception type", s->exception == excs[i], s->exception);
        EXC_CHECK("two codes", s->code_cnt == 2, s->code_cnt);
        if (ident) {
            EXC_CHECK("the thread descriptor is the faulting thread, moved as a send right", s->thread_is_faulter == 1 && s->thread_disp == MACH_MSG_TYPE_MOVE_SEND, s->thread_disp);
            EXC_CHECK("the task descriptor is this task, moved as a send right", s->task_is_self == 1 && s->task_disp == MACH_MSG_TYPE_MOVE_SEND, s->task_disp);
        } else {
            EXC_CHECK("no descriptors", s->thread_is_faulter == -1 && s->task_is_self == -1, s->thread_is_faulter);
        }
        if (state) {
            EXC_CHECK("the state flavor is the architecture's thread state", s->flavor == EXC_FLAVOR, s->flavor);
            EXC_CHECK("the state count is that flavor's count", s->state_cnt == EXC_STATE_COUNT, s->state_cnt);
        }
        EXC_CHECK("the default trailer, 8 bytes", s->trailer_size == 8, s->trailer_size);
#undef EXC_CHECK
    }
    check("access: code 0 is KERN_PROTECTION_FAILURE", seen[0].code[0] == KERN_PROTECTION_FAILURE, seen[0].code[0]);
    check(codes64 ? "access: code 1 is the whole 64-bit address" : "access: code 1 is the low 32 bits of the address",
          seen[0].code[1] == (codes64 ? (long long)EXC_PAGE_ADDR : (long long)(EXC_PAGE_ADDR & 0xffffffffUL)), seen[0].code[1]);
    check("undefined instruction: code 0 is 1", seen[1].code[0] == 1, seen[1].code[0]);
    value("breakpoint: code 0", seen[2].code[0]);
    value("message size, access", seen[0].size);
}

/* The exceptions of the classic range, 1 to 9 (access, instruction, arithmetic, emulation, software, breakpoint, syscall, Mach syscall,
 * RPC alert): a fresh task has no port for any of them on any architecture observed; for the ones above (resource, guard, crash)
 * it may have, inherited from where it was started. */
#define EXC_MASK_CLASSIC 0x3feU

/* One entry of what task_/thread_get_exception_ports returns. */
typedef struct {
    exception_mask_t mask;
    mach_port_t port;
    exception_behavior_t behavior;
    thread_state_flavor_t flavor;
} exc_entry_t;
/* get_exception_ports on a task or thread port, for `mask`; the entries in `out` (at most 14); returns their count, or
 * -1 - the kern_return_t when the call failed. */
static inline int exc_get(mach_port_t target, int is_thread, exception_mask_t mask, exc_entry_t out[14]) {
    exception_mask_t m[EXC_TYPES_COUNT];
    mach_port_t p[EXC_TYPES_COUNT];
    exception_behavior_t b[EXC_TYPES_COUNT];
    thread_state_flavor_t f[EXC_TYPES_COUNT];
    mach_msg_type_number_t n = EXC_TYPES_COUNT;
    kern_return_t kr = is_thread ? thread_get_exception_ports(target, mask, m, &n, p, b, f) : task_get_exception_ports(target, mask, m, &n, p, b, f);
    if (kr) return -1 - kr;
    for (unsigned i = 0; i < n; i++) {
        out[i].mask = m[i];
        out[i].port = p[i];
        out[i].behavior = b[i];
        out[i].flavor = f[i];
    }
    return (int)n;
}
/* The entry that covers the exception bit `bit` (EXC_MASK_*), or NULL. */
static inline exc_entry_t *exc_entry_for(exc_entry_t *e, int n, exception_mask_t bit) {
    for (int i = 0; i < n; i++)
        if (e[i].mask & bit) return &e[i];
    return NULL;
}
/* A thread of this program that waits until released, with its port in *thread_out: for the tests of the thread calls. */
static volatile int exc_release;
static mach_port_t exc_waiter_port;
static void *exc_waiter(void *a) {
    (void)a;
    exc_waiter_port = mach_thread_self();
    while (!exc_release) usleep(1000);
    return NULL;
}
static inline pthread_t exc_start_waiter(mach_port_t *thread_out) {
    pthread_t t;
    exc_release = 0;
    exc_waiter_port = MACH_PORT_NULL;
    pthread_create(&t, NULL, exc_waiter, NULL);
    while (!exc_waiter_port) usleep(1000);
    *thread_out = exc_waiter_port;
    return t;
}
static inline void exc_stop_waiter(pthread_t t) {
    exc_release = 1;
    pthread_join(t, NULL);
}

/* A child of this program for the tests that let a process die: runs argv[0] again with mode and argument. */
static inline pid_t exc_start_self(const char *mode, const char *arg) {
    char *argv[4] = {poly_argv[0], (char *)mode, (char *)arg, NULL};
    pid_t pid = -1;
    return posix_spawn(&pid, poly_argv[0], NULL, NULL, argv, environ) ? -1 : pid;
}
/* The ending of a child: its exit status, or 1000 + the signal that ended it, or -1. */
static inline int exc_wait(pid_t pid) {
    int st = 0;
    if (waitpid(pid, &st, 0) != pid) return -1;
    return WIFEXITED(st) ? WEXITSTATUS(st) : 1000 + WTERMSIG(st);
}
static inline const char *exc_child_arg(void) { return poly_argc > 2 ? poly_argv[2] : ""; }

#endif
