/* A message longer than msgsz: without MSG_NOERROR the call fails with
 * E2BIG and the message stays at its place in the queue (qnum, cbytes,
 * lrpid and rtime unchanged; nothing is written to the buffer). With
 * MSG_NOERROR (010000) the first msgsz bytes are copied, the rest of the
 * text is dropped, the return value is msgsz and the whole message
 * leaves the queue (cbytes drops by its full size). Bytes past msgsz in
 * the buffer are not touched. An E2BIG message does not block messages
 * behind it from being taken by type.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

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
#define MSG_COPY 040000
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
static __attribute__((unused)) struct k_msg in, out;

static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
static volatile int caught;
static __attribute__((unused)) void on_signal(int sig) {
    (void)sig;
    caught++;
}
/* Fork a receiver: msgrcv(id, size, type, flg) with a handler for `sig`
 * (0: none) installed with `sflags`. It exits with 100 + errno on
 * failure, else with mtype * 10 + the returned length. */
static __attribute__((unused)) i64 receiver(i64 id, u64 size, i64 type, i64 flg, int sig, u64 sflags) {
    i64 pid = fork();
    if (pid == 0) {
        if (sig) sig_handler(sig, on_signal, sflags);
        static struct k_msg m;
        i64 r = msgrcv(id, &m, size, type, flg);
        exit_group(r < 0 ? (int)(100 - r) : (int)(m.mtype * 10 + r));
    }
    return pid;
}

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);

    in.mtype = 4;
    memcpy(in.mtext, "abcdefgh", 8);
    msgsnd(id, &in, 8, 0);
    send_fill(id, 6, 'x', 2, 0);

    memset(&out, 'Z', sizeof out);
    i64 r = msgrcv(id, &out, 7, 0, IPC_NOWAIT);
    check("e2big", r == -E2BIG, r);
    check("e2big_type_untouched", out.mtype == 0x5a5a5a5a5a5a5a5aL, out.mtype);
    check("e2big_text_untouched", out.mtext[0] == 'Z', out.mtext[0]);
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    check("e2big_qnum_2", ds.qnum == 2, (i64)ds.qnum);
    check("e2big_cbytes_10", ds.cbytes == 10, (i64)ds.cbytes);
    check("e2big_lrpid_0", ds.lrpid == 0, ds.lrpid);
    check("e2big_rtime_0", ds.rtime == 0, ds.rtime);
    r = msgrcv(id, &out, 0, 4, IPC_NOWAIT);
    check("e2big_size_0", r == -E2BIG, r);
    r = msgrcv(id, &out, 1, -10, IPC_NOWAIT);
    check("e2big_lowest", r == -E2BIG, r);

    /* The message behind it can still be taken by type. */
    r = msgrcv(id, &out, 7, 6, IPC_NOWAIT);
    check("behind_by_type", r == 2 && out.mtype == 6, r);
    /* And the long one is still first. */
    r = msgrcv(id, &out, 8, 0, IPC_NOWAIT | MSG_COPY);
    check("still_first", r == 8 && out.mtype == 4, r);

    memset(&out, 'Z', sizeof out);
    r = msgrcv(id, &out, 3, 0, IPC_NOWAIT | MSG_NOERROR);
    check("noerror_returns_3", r == 3, r);
    check("noerror_type", out.mtype == 4, out.mtype);
    check("noerror_text", mem_eq(out.mtext, "abc", 3), 0);
    check("noerror_rest_untouched", out.mtext[3] == 'Z' && out.mtext[7] == 'Z', out.mtext[3]);
    check("noerror_removed", msq_qnum(id) == 0 && msq_cbytes(id) == 0, msq_cbytes(id));

    send_fill(id, 5, 'q', 6, 0);
    memset(&out, 'Z', sizeof out);
    r = msgrcv(id, &out, 0, 5, IPC_NOWAIT | MSG_NOERROR);
    check("noerror_size_0", r == 0 && out.mtype == 5 && out.mtext[0] == 'Z', r);
    check("noerror_size_0_removed", msq_qnum(id) == 0, msq_qnum(id));

    /* MSG_NOERROR with a buffer that is big enough changes nothing. */
    send_fill(id, 5, 'q', 6, 0);
    r = msgrcv(id, &out, 6, 5, IPC_NOWAIT | MSG_NOERROR);
    check("noerror_fits", r == 6, r);
    send_fill(id, 5, 'q', 6, 0);
    r = msgrcv(id, &out, 100, 5, IPC_NOWAIT | MSG_NOERROR);
    check("noerror_bigger", r == 6, r);

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
