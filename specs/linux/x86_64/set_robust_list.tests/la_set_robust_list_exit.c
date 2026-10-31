/* What the robust list is for: when a thread ends (here: a CLONE_VM
 * task, and a fork child whose lock words are in MAP_SHARED memory), the
 * kernel walks its list (head.list.next -> entry -> ... -> back to the
 * head; each entry's lock word is at entry + head.futex_offset) and, for
 * every lock word whose owner bits (FUTEX_TID_MASK 0x3fffffff) equal
 * the ending thread's id, replaces the word by FUTEX_OWNER_DIED
 * (0x40000000) plus the FUTEX_WAITERS bit (0x80000000) if it was set:
 * the owner id is cleared. Lock words owned by another id are left
 * alone, and so are words not on the list. The entry named by
 * list_op_pending is handled the same way. Without a registered list
 * nothing is changed.
 */
#include "a_sys.h"

#define SYS_set_robust_list 273
/* linux/futex.h */
#define FUTEX_WAITERS 0x80000000U
#define FUTEX_OWNER_DIED 0x40000000U
#define FUTEX_TID_MASK 0x3fffffffU

/* struct robust_list, struct robust_list_head (linux/futex.h) */
struct k_robust_list {
    struct k_robust_list *next;
};
struct k_robust_list_head {
    struct k_robust_list list;
    long futex_offset;
    struct k_robust_list *list_op_pending;
};

/* A lock: the list link, then the lock word. */
struct lock {
    struct k_robust_list link;
    u32 word;
    u32 pad;
};

struct shared {
    struct k_robust_list_head head;
    struct lock owned, waiters, foreign, pending, unlisted;
    i32 tid;
};

static volatile i32 done;

/* Build a list owned by the calling task, register it and return. */
static void build(struct shared *s, i32 tid) {
    s->tid = tid;
    s->head.list.next = &s->owned.link;
    s->owned.link.next = &s->waiters.link;
    s->waiters.link.next = &s->foreign.link;
    s->foreign.link.next = &s->head.list;
    s->head.futex_offset = 8;
    s->head.list_op_pending = &s->pending.link;
    s->owned.word = (u32)tid;
    s->waiters.word = (u32)tid | FUTEX_WAITERS;
    s->foreign.word = 12345;
    s->pending.word = (u32)tid;
    s->unlisted.word = (u32)tid;
    syscall2(SYS_set_robust_list, &s->head, 24);
}

static int task(void *arg) {
    struct shared *s = arg;
    build(s, (i32)syscall0(SYS_gettid));
    return 0;
}

static void verify(const char *what, struct shared *s) {
    char name[64];
    char *p = str_cpy(name, what);
    str_cpy(p, "_owned_marked");
    expect(name, s->owned.word, FUTEX_OWNER_DIED);
    str_cpy(p, "_waiters_kept");
    expect(name, s->waiters.word, FUTEX_OWNER_DIED | FUTEX_WAITERS);
    str_cpy(p, "_foreign_untouched");
    expect(name, s->foreign.word, 12345);
    str_cpy(p, "_pending_marked");
    expect(name, s->pending.word, FUTEX_OWNER_DIED);
    str_cpy(p, "_unlisted_untouched");
    expect(name, s->unlisted.word, (u32)s->tid);
}

static struct shared thread_data;

int test_main(void) {
    a_block_sigchld();
    long pid = a_spawn(0, &done, task, &thread_data);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    check("spawned", pid > 0, pid);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    expect("thread_tid_matches", thread_data.tid, pid);
    verify("thread", &thread_data);

    struct shared *s = (struct shared *)a_shared();
    i64 child = a_fork_sync();
    if (child == 0) {
        build(s, (i32)syscall0(SYS_getpid));
        syscall1(SYS_exit, 0);
    }
    syscall4(SYS_wait4, child, 0, 0, 0);
    expect("process_tid_matches", s->tid, child);
    verify("process", s);

    /* Without a registered list nothing happens. */
    child = a_fork_sync();
    if (child == 0) {
        syscall2(SYS_set_robust_list, 0, 24);
        s->tid = (i32)syscall0(SYS_getpid);
        s->owned.word = (u32)s->tid;
        syscall1(SYS_exit, 0);
    }
    syscall4(SYS_wait4, child, 0, 0, 0);
    expect("no_list_untouched", s->owned.word, (u32)s->tid);
    return finish();
}
