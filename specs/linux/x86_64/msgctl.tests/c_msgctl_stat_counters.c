/* IPC_STAT after msgsnd and msgrcv: qnum counts the queued messages,
 * cbytes their text bytes (mtype not counted), lspid is the pid of the
 * last sender and lrpid of the last receiver (here a child), stime and
 * rtime the times of the last send and receive. A failed send or receive
 * changes none of them; msgctl IPC_STAT leaves ctime alone.
 * poly-diff: syscalls=off (two processes; queue ids vary)
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

int test_main(void) {
    i64 me = getpid();
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("create_ok", id >= 0, id);
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    i64 ctime0 = ds.ctime;

    i64 t0 = now_sec();
    check("send_3", msq_send(id, 1, 3, 0) == 0, 0);
    check("send_10", msq_send(id, 2, 10, 0) == 0, 0);
    check("send_0", msq_send(id, 3, 0, 0) == 0, 0);
    i64 t1 = now_sec();
    i64 r = msq_stat(id, &ds);
    check("stat_ok", r == 0, r);
    check("qnum_3", ds.qnum == 3, (i64)ds.qnum);
    check("cbytes_13", ds.cbytes == 13, (i64)ds.cbytes);
    check("lspid_me", ds.lspid == me, ds.lspid);
    check("lrpid_still_0", ds.lrpid == 0, ds.lrpid);
    check("stime_now", ds.stime >= t0 && ds.stime <= t1, ds.stime - t0);
    check("rtime_still_0", ds.rtime == 0, ds.rtime);
    check("ctime_unchanged", ds.ctime == ctime0, ds.ctime - ctime0);

    i64 child = fork();
    if (child == 0) {
        i64 n = msq_recv(id, 0);
        exit_group(n == 3 ? 0 : 1);
    }
    check("child_received", wait_exit(child) == 0, 0);
    i64 t2 = now_sec();
    r = msq_stat(id, &ds);
    check("stat2_ok", r == 0, r);
    check("qnum_2", ds.qnum == 2, (i64)ds.qnum);
    check("cbytes_10", ds.cbytes == 10, (i64)ds.cbytes);
    check("lrpid_child", ds.lrpid == child, ds.lrpid - child);
    check("lspid_still_me", ds.lspid == me, ds.lspid);
    check("rtime_now", ds.rtime >= t1 && ds.rtime <= t2, ds.rtime - t1);

    struct k_msqid_ds before = ds;
    r = msq_recv(id, 0); /* the 10-byte message */
    check("recv_10", r == 10, r);
    r = msq_recv(id, 0); /* the empty message */
    check("recv_0", r == 0, r);
    r = msq_stat(id, &ds);
    check("lrpid_me", ds.lrpid == me, ds.lrpid);
    check("empty_qnum", ds.qnum == 0, (i64)ds.qnum);
    check("empty_cbytes", ds.cbytes == 0, (i64)ds.cbytes);

    before = ds;
    r = msq_recv(id, IPC_NOWAIT);
    check("recv_empty_enomsg", r == -ENOMSG, r);
    r = msq_send(id, 0, 1, IPC_NOWAIT);
    check("send_type0_einval", r == -EINVAL, r);
    msq_stat(id, &ds);
    check("failed_calls_change_nothing", mem_eq(&before, &ds, sizeof ds), 0);

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
