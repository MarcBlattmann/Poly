/* IPC_RMID wakes every process sleeping on the queue: blocked receivers
 * (msgrcv on an empty queue) and blocked senders (msgsnd on a full
 * queue) all return EIDRM; the removal itself returns 0 at once.
 * poly-diff: syscalls=off (several processes; their order varies)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
/* asm-generic/errno.h */
#define ENOMSG 42
#define EIDRM 43
/* linux/ipc.h, linux/msg.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
#define IPC_INFO 3
#define IPC_64 0x0100
#define MSG_STAT 11
#define MSG_INFO 12
#define MSG_STAT_ANY 13

/* struct msqid64_ds (asm-generic/msgbuf.h, struct ipc64_perm from
 * asm-generic/ipcbuf.h) on x86-64: 120 bytes. */
struct k_msqid_ds {
    int key;
    unsigned uid, gid, cuid, cgid;
    unsigned short mode, pad1;
    unsigned short seq, pad2;
    unsigned pad3;
    u64 unused1, unused2;
    i64 stime, rtime, ctime;
    u64 cbytes, qnum, qbytes;
    int lspid, lrpid;
    u64 unused4, unused5;
};

/* struct msginfo (linux/msg.h): 7 ints and a short, 32 bytes. */
struct k_msginfo {
    int msgpool, msgmap, msgmax, msgmnb, msgmni, msgssz, msgtql;
    unsigned short msgseg;
    unsigned short pad;
};

/* struct msgbuf (linux/msg.h) with a small text. */
struct k_msg {
    i64 mtype;
    char mtext[64];
};

static __attribute__((unused)) i64 msgget(i64 key, i64 flg) { return syscall2(SYS_msgget, key, flg); }
static __attribute__((unused)) i64 msgctl(i64 id, i64 cmd, const void *buf) { return syscall3(SYS_msgctl, id, cmd, buf); }
static __attribute__((unused)) i64 msq_rm(i64 id) { return msgctl(id, IPC_RMID, 0); }
static __attribute__((unused)) i64 msq_stat(i64 id, struct k_msqid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return msgctl(id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 msq_send(i64 id, i64 type, u64 len, i64 flg) {
    struct k_msg m;
    memset(&m, 'm', sizeof m);
    m.mtype = type;
    return syscall4(SYS_msgsnd, id, &m, len, flg);
}
static __attribute__((unused)) i64 msq_recv(i64 id, i64 flg) {
    struct k_msg m;
    memset(&m, 0, sizeof m);
    return syscall5(SYS_msgrcv, id, &m, sizeof m.mtext, 0, flg);
}
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x50000000 | (tag << 22) | (int)getpid()); }

static volatile unsigned *ready;

int test_main(void) {
    ready = shared_page(4096);
    check("shared_ok", ready != 0, 0);

    /* Two receivers on an empty queue. */
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("create_ok", id >= 0, id);
    i64 kids[2];
    for (int i = 0; i < 2; i++) {
        kids[i] = fork();
        if (kids[i] == 0) {
            __atomic_add_fetch(ready, 1, __ATOMIC_SEQ_CST);
            i64 r = msq_recv(id, 0);
            exit_group(r == -EIDRM ? 0 : 1);
        }
    }
    while (__atomic_load_n(ready, __ATOMIC_SEQ_CST) < 2) sleep_ms(1);
    sleep_ms(100); /* let them go to sleep in msgrcv */
    i64 r = msq_rm(id);
    check("rmid_with_receivers_ok", r == 0, r);
    check("receiver1_eidrm", wait_exit(kids[0]) == 0, 0);
    check("receiver2_eidrm", wait_exit(kids[1]) == 0, 0);

    /* A sender on a full queue. */
    *ready = 0;
    id = msgget(IPC_PRIVATE, 0600);
    check("create2_ok", id >= 0, id);
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    ds.qbytes = 8;
    check("qbytes_8", msgctl(id, IPC_SET, &ds) == 0, 0);
    check("fill", msq_send(id, 1, 8, IPC_NOWAIT) == 0, 0);
    r = msq_send(id, 1, 1, IPC_NOWAIT);
    check("full_eagain", r == -EAGAIN, r);
    i64 sender = fork();
    if (sender == 0) {
        __atomic_store_n(ready, 1, __ATOMIC_SEQ_CST);
        r = msq_send(id, 1, 4, 0);
        exit_group(r == -EIDRM ? 0 : 1);
    }
    while (__atomic_load_n(ready, __ATOMIC_SEQ_CST) < 1) sleep_ms(1);
    sleep_ms(100);
    r = msq_rm(id);
    check("rmid_with_sender_ok", r == 0, r);
    check("sender_eidrm", wait_exit(sender) == 0, 0);
    return finish();
}
