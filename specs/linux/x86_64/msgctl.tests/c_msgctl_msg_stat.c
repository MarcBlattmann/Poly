/* MSG_STAT and MSG_STAT_ANY take a queue index (only its low 15 bits
 * count, so an id works too) instead of an id, fill struct msqid64_ds
 * exactly as IPC_STAT does, and return the queue's full id. An index
 * with no queue is EINVAL; NULL is EFAULT. MSG_STAT needs read
 * permission (EACCES), MSG_STAT_ANY does not.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
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
    int key = own_key(9);
    i64 id = msgget(key, 0640 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    i64 idx = id & 0x7fff;
    check("send_ok", msq_send(id, 4, 33, 0) == 0, 0);

    struct k_msqid_ds want, got;
    msq_stat(id, &want);
    memset(&got, 0xa5, sizeof got);
    i64 r = msgctl(idx, MSG_STAT, &got);
    check("msg_stat_returns_id", r == id, r - id);
    check("msg_stat_same_as_ipc_stat", mem_eq(&got, &want, sizeof got), 0);
    check("msg_stat_key", got.key == key, got.key);
    check("msg_stat_cbytes", got.cbytes == 33, (i64)got.cbytes);

    memset(&got, 0x5a, sizeof got);
    r = msgctl(idx, MSG_STAT_ANY, &got);
    check("msg_stat_any_returns_id", r == id, r - id);
    check("msg_stat_any_same", mem_eq(&got, &want, sizeof got), 0);

    r = msgctl(id, MSG_STAT, &got);
    check("full_id_as_index", r == id, r - id);
    r = msgctl(idx | (5 << 15), MSG_STAT, &got);
    check("index_high_bits_ignored", r == id, r - id);

    r = msgctl(idx, MSG_STAT, 0);
    check("msg_stat_null_efault", r == -EFAULT, r);
    r = msgctl(idx, MSG_STAT_ANY, 0);
    check("msg_stat_any_null_efault", r == -EFAULT, r);

    check("rm_ok", msq_rm(id) == 0, 0);
    r = msgctl(idx, MSG_STAT, &got);
    check("removed_index_einval", r == -EINVAL, r);
    r = msgctl(idx, MSG_STAT_ANY, &got);
    check("removed_index_any_einval", r == -EINVAL, r);
    return finish();
}
