/* Restartable sequences: while a registered thread runs inside the
 * critical section described by the rseq_cs field of its area
 * (struct rseq_cs {u32 version 0; u32 flags 0; u64 start_ip; u64
 * post_commit_offset; u64 abort_ip}: the section is start_ip up to, not
 * including, start_ip + post_commit_offset), being preempted or getting
 * a signal makes the kernel move the thread to abort_ip (a handler runs
 * first and returns there) and clear rseq_cs to 0. The 4 bytes before
 * abort_ip must hold the signature given at registration; if they do
 * not, the thread is killed by SIGSEGV instead. Outside the section the
 * kernel also clears rseq_cs when it looks at it, so the section here
 * leaves early (returning 0) when it finds rseq_cs already 0 and is
 * entered again; a periodic timer signal guarantees an abort.
 * poly-diff: syscalls=off (the number of timer signals depends on timing)
 */
#include "a_sys.h"

#define SYS_rseq 334
#define SIG 0x53053053

struct k_rseq {
    u32 cpu_id_start;
    u32 cpu_id;
    u64 rseq_cs;
    u32 flags;
    u32 node_id;
    u32 mm_cid;
    u32 pad;
} __attribute__((aligned(32)));

/* struct rseq_cs (linux/rseq.h) */
struct k_rseq_cs {
    u32 version;
    u32 flags;
    u64 start_ip;
    u64 post_commit_offset;
    u64 abort_ip;
} __attribute__((aligned(32)));

static struct k_rseq rs;
static struct k_rseq_cs cs, bad_cs;
static volatile u64 counter;
static volatile i32 signals;

/* la_cs_run(counter, rseq_cs_field): spins inside the critical section
 * [la_cs_start, la_cs_end) until aborted (returns 1 through la_cs_abort,
 * 2 through la_cs_bad_abort) or until it finds the rseq_cs field 0
 * (returns 0). */
long la_cs_run(volatile u64 *counter, volatile u64 *field);
extern char la_cs_start[], la_cs_end[], la_cs_abort[], la_cs_bad_abort[];
__asm__(".text\n"
        ".global la_cs_run, la_cs_start, la_cs_end, la_cs_abort, la_cs_bad_abort\n"
        "la_cs_run:\n"
        "la_cs_start:\n"
        "1:  cmpq $0, (%rsi)\n"
        "    je la_cs_end\n"
        "    incq (%rdi)\n"
        "    jmp 1b\n"
        "la_cs_end:\n"
        "    xor %eax, %eax\n"
        "    ret\n"
        "    .long 0x53053053\n"
        "la_cs_abort:\n"
        "    mov $1, %eax\n"
        "    ret\n"
        "    .long 0x22222222\n"
        "la_cs_bad_abort:\n"
        "    mov $2, %eax\n"
        "    ret\n");

void la_restorer(void);
__asm__(".text\n"
        ".global la_restorer\n"
        "la_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static void on_alarm(int sig) {
    (void)sig;
    signals++;
}

static void timer(i64 usec) {
    struct a_itimerval it = { { 0, usec }, { 0, usec } };
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
}

static long run(struct k_rseq_cs *c) {
    long r;
    do {
        rs.rseq_cs = (u64)c;
        r = la_cs_run(&counter, &rs.rseq_cs);
    } while (r == 0);
    return r;
}

int test_main(void) {
    struct a_sigaction sa = { (u64)on_alarm, SA_RESTORER, (u64)la_restorer, 0 };
    expect("sigaction", syscall4(SYS_rt_sigaction, SIGALRM, &sa, 0, 8), 0);
    expect("register", syscall4(SYS_rseq, &rs, 32, 0, SIG), 0);

    cs.start_ip = (u64)la_cs_start;
    cs.post_commit_offset = (u64)(la_cs_end - la_cs_start);
    cs.abort_ip = (u64)la_cs_abort;
    bad_cs = cs;
    bad_cs.abort_ip = (u64)la_cs_bad_abort;

    timer(5000);
    long r = run(&cs);
    u64 cleared = rs.rseq_cs;
    u64 start = a_rdtsc();
    while (!signals && a_rdtsc() - start < A_SPIN_LIMIT) a_relax();
    timer(0);
    expect("aborted_to_abort_ip", r, 1);
    expect("rseq_cs_cleared", (i64)cleared, 0);
    check("timer_signal_handled", signals > 0, signals);

    /* A wrong signature before abort_ip: SIGSEGV. */
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        timer(5000);
        run(&bad_cs);
        syscall1(SYS_exit, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    check("bad_signature_sigsegv", A_WIFSIGNALED(status) && A_WTERMSIG(status) == SIGSEGV, status);
    expect("unregister", syscall4(SYS_rseq, &rs, 32, 1, SIG), 0);
    return finish();
}
