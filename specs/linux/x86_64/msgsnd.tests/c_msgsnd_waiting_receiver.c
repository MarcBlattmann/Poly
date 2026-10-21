/* A send to a queue with a receiver sleeping in msgrcv for a matching
 * type hands the message straight over: when msgsnd returns, the queue
 * is already empty again (qnum 0), lspid is the sender, lrpid the
 * receiver and rtime is set. A sleeping receiver that does not match the
 * type leaves the message queued. A sleeping receiver whose buffer is too
 * small (no MSG_NOERROR) fails with E2BIG and the message stays queued.
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
/* A child that waits in msgrcv(type, size) and exits 0 when it gets a
 * message of `want_len` bytes and type `want_type`; 100+errno on error. */
static i64 receiver(i64 id, i64 type, u64 size, i64 want_len, i64 want_type) {
    i64 pid = fork();
    if (pid == 0) {
        static struct k_msg m;
        i64 r = msgrcv(id, &m, size, type, 0);
        if (r < 0) exit_group(100 + (int)-r);
        exit_group(r == want_len && m.mtype == want_type ? 0 : 1);
    }
    return pid;
}

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);

    i64 pid = receiver(id, 0, 100, 3, 5);
    sleep_ms(100);
    i64 r = send_fill(id, 5, 'h', 3, IPC_NOWAIT);
    check("send_ok", r == 0, r);
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    check("handed_over_qnum_0", ds.qnum == 0, (i64)ds.qnum);
    check("handed_over_cbytes_0", ds.cbytes == 0, (i64)ds.cbytes);
    check("lspid_sender", ds.lspid == getpid(), ds.lspid);
    check("lrpid_receiver", ds.lrpid == pid, ds.lrpid);
    check("rtime_set", ds.rtime > 0 && ds.stime > 0, 0);
    check("receiver_got_it", wait_exit(pid) == 0, 0);

    /* A receiver for type 2 does not take a type 1 message. */
    pid = receiver(id, 2, 100, 3, 2);
    sleep_ms(100);
    r = send_fill(id, 1, 'a', 3, 0);
    check("type_1_ok", r == 0, r);
    check("type_1_queued", msq_qnum(id) == 1, msq_qnum(id));
    r = send_fill(id, 2, 'b', 3, 0);
    check("type_2_ok", r == 0, r);
    check("type_2_taken", msq_qnum(id) == 1, msq_qnum(id));
    check("receiver_2_got_type_2", wait_exit(pid) == 0, 0);
    struct k_msg m;
    r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("type_1_still_there", r == 3 && m.mtype == 1, m.mtype);

    /* A receiver with a 2-byte buffer and a 3-byte message. */
    pid = receiver(id, 0, 2, 0, 0);
    sleep_ms(100);
    r = send_fill(id, 4, 'c', 3, 0);
    check("send_to_small_ok", r == 0, r);
    i64 code = wait_exit(pid);
    check("small_receiver_e2big", code == 100 + E2BIG, code);
    check("message_stays", msq_qnum(id) == 1 && msq_cbytes(id) == 3, msq_qnum(id));

    msq_rm(id);
    return finish();
}
