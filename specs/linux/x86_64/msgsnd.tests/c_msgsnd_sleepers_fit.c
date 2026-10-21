/* Several sleeping senders: when room is made, each one whose message
 * now fits proceeds, in no guaranteed order. A sleeper whose message
 * still does not fit keeps sleeping while a later, smaller one goes
 * ahead (no head-of-line blocking). A send with IPC_NOWAIT that fits
 * succeeds even while others sleep.
 * poly-diff: syscalls=off (several processes, timing-dependent polling)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EPERM 1
#define EINTR 4
#define E2BIG 7
#define EAGAIN 11
#define EACCES 13
#define ENOMSG 42
#define EIDRM 43
/* linux/ipc.h, linux/msg.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_NOWAIT 04000
#define MSG_NOERROR 010000
#define MSG_EXCEPT 020000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2

/* struct msqid64_ds (asm-generic/msgbuf.h, struct ipc64_perm from
 * asm-generic/ipcbuf.h) on x86-64. */
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

/* struct msgbuf (linux/msg.h): a long type, then the text. */
struct k_msg {
    i64 mtype;
    char mtext[8200];
};

static __attribute__((unused)) i64 msgget(i64 key, i64 flg) { return syscall2(SYS_msgget, key, flg); }
static __attribute__((unused)) i64 msgsnd(i64 id, const void *msg, u64 size, i64 flg) {
    return syscall4(SYS_msgsnd, id, msg, size, flg);
}
static __attribute__((unused)) i64 msgrcv(i64 id, void *msg, u64 size, i64 type, i64 flg) {
    return syscall5(SYS_msgrcv, id, msg, size, type, flg);
}
static __attribute__((unused)) i64 msq_rm(i64 id) { return syscall3(SYS_msgctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 msq_stat(i64 id, struct k_msqid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_msgctl, id, IPC_STAT, ds);
}
/* IPC_SET with the queue's current settings but the given qbytes and mode. */
static __attribute__((unused)) i64 msq_set(i64 id, u64 qbytes, int mode) {
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    ds.qbytes = qbytes;
    ds.mode = (unsigned short)mode;
    return syscall3(SYS_msgctl, id, IPC_SET, &ds);
}
static __attribute__((unused)) i64 msq_qnum(i64 id) {
    struct k_msqid_ds ds;
    return msq_stat(id, &ds) == 0 ? (i64)ds.qnum : -1;
}
static __attribute__((unused)) i64 msq_cbytes(i64 id) {
    struct k_msqid_ds ds;
    return msq_stat(id, &ds) == 0 ? (i64)ds.cbytes : -1;
}
/* Send `size` bytes of `fill` with type `type`. */
static __attribute__((unused)) i64 send_fill(i64 id, i64 type, char fill, u64 size, i64 flg) {
    static struct k_msg m;
    m.mtype = type;
    memset(m.mtext, fill, size < sizeof m.mtext ? size : sizeof m.mtext);
    return msgsnd(id, &m, size, flg);
}
/* Poll qnum until it is `want`; 0 after ~2 s. */
static __attribute__((unused)) int wait_qnum(i64 id, i64 want) {
    for (int i = 0; i < 2000; i++) {
        if (msq_qnum(id) == want) return 1;
        sleep_ms(1);
    }
    return 0;
}
static i64 sender(i64 id, i64 type, u64 size) {
    i64 pid = fork();
    if (pid == 0) {
        i64 r = send_fill(id, type, 's', size, 0);
        exit_group(r == 0 ? 0 : 100 + (int)-r);
    }
    return pid;
}
static int still_running(i64 pid) {
    int st = 0;
    return syscall4(SYS_wait4, pid, &st, WNOHANG, 0) == 0;
}

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    msq_set(id, 10, 0600);
    send_fill(id, 1, 'f', 5, 0);
    send_fill(id, 1, 'f', 5, 0);

    i64 big = sender(id, 2, 8);
    sleep_ms(50);
    i64 small = sender(id, 3, 2);
    sleep_ms(100);
    check("both_sleeping", still_running(big) && still_running(small), 0);

    /* 5 bytes free: the later 2-byte message fits, the 8-byte one does not. */
    struct k_msg m;
    msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("small_proceeds", wait_exit(small) == 0, 0);
    check("big_still_sleeping", still_running(big), 0);
    check("cbytes_7", msq_cbytes(id) == 7, msq_cbytes(id));
    i64 r = send_fill(id, 4, 'n', 3, IPC_NOWAIT);
    check("nowait_fits_past_sleeper", r == 0, r);
    r = send_fill(id, 4, 'n', 1, IPC_NOWAIT);
    check("nowait_full", r == -EAGAIN, r);

    /* Empty the queue: the big one goes. */
    while (msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT) >= 0) {
    }
    check("big_proceeds", wait_exit(big) == 0, 0);
    r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("big_message", r == 8 && m.mtype == 2, r);

    /* Three sleepers of 5 bytes on a full queue; making room for two lets
     * exactly two through. */
    send_fill(id, 1, 'f', 10, 0);
    i64 a = sender(id, 5, 5), b = sender(id, 5, 5), c = sender(id, 5, 5);
    sleep_ms(100);
    msgrcv(id, &m, sizeof m.mtext, 1, IPC_NOWAIT);
    check("two_through", wait_qnum(id, 2), msq_qnum(id));
    sleep_ms(50);
    check("third_waits", msq_qnum(id) == 2 && msq_cbytes(id) == 10, msq_qnum(id));
    /* Reap the two that went through and note their status. */
    i64 kids[3] = { a, b, c };
    int running = 0, through_ok = 1;
    i64 left = -1;
    for (int i = 0; i < 3; i++) {
        int st = 0;
        if (syscall4(SYS_wait4, kids[i], &st, WNOHANG, 0) == 0) {
            running++;
            left = kids[i];
        } else if (!W_EXITED(st) || W_EXITCODE(st) != 0) {
            through_ok = 0;
        }
    }
    check("one_still_sleeping", running == 1, running);
    check("two_returned_0", through_ok, 0);
    msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("last_done", wait_exit(left) == 0, 0);
    check("final_qnum", msq_qnum(id) == 2, msq_qnum(id));

    msq_rm(id);
    return finish();
}
