/* Ids: forty IPC_PRIVATE queues get forty distinct non-negative ids, and
 * each id addresses its own queue (a message sent on one is only on that
 * one). An id just removed is not handed out by the next msgget.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
/* linux/ipc.h, linux/msg.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
/* asm-generic/errno.h */
#define ENOMSG 42
#define EIDRM 43

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

/* struct msgbuf (linux/msg.h) with a small text. */
struct k_msg {
    i64 mtype;
    char mtext[16];
};

static __attribute__((unused)) i64 msgget(i64 key, i64 flg) { return syscall2(SYS_msgget, key, flg); }
static __attribute__((unused)) i64 msq_rm(i64 id) { return syscall3(SYS_msgctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 msq_stat(i64 id, struct k_msqid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_msgctl, id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 msq_send(i64 id, i64 type, const char *text, u64 len) {
    struct k_msg m;
    memset(&m, 0, sizeof m);
    m.mtype = type;
    memcpy(m.mtext, text, len);
    return syscall4(SYS_msgsnd, id, &m, len, IPC_NOWAIT);
}
static __attribute__((unused)) i64 msq_recv(i64 id, struct k_msg *m) {
    memset(m, 0, sizeof *m);
    return syscall5(SYS_msgrcv, id, m, sizeof m->mtext, 0, IPC_NOWAIT);
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x40000000 | (tag << 22) | (int)getpid()); }
#define N 40
int test_main(void) {
    i64 ids[N];
    int ok = 1, distinct = 1;
    for (int i = 0; i < N; i++) {
        ids[i] = msgget(IPC_PRIVATE, 0600);
        if (ids[i] < 0) ok = 0;
        for (int j = 0; j < i; j++)
            if (ids[j] == ids[i]) distinct = 0;
    }
    check("all_ok", ok, 0);
    check("all_distinct", distinct, 0);

    i64 r = msq_send(ids[17], 5, "q17", 3);
    check("send_ok", r == 0, r);
    int others_empty = 1;
    struct k_msqid_ds ds;
    for (int i = 0; i < N; i++) {
        if (i == 17) continue;
        if (msq_stat(ids[i], &ds) != 0 || ds.qnum != 0) others_empty = 0;
    }
    check("others_empty", others_empty, 0);
    r = msq_stat(ids[17], &ds);
    check("target_has_one", r == 0 && ds.qnum == 1 && ds.cbytes == 3, (i64)ds.qnum);

    i64 old = ids[5];
    msq_rm(old);
    i64 fresh = msgget(IPC_PRIVATE, 0600);
    check("fresh_ok", fresh >= 0, fresh);
    check("removed_id_not_reused", fresh != old, fresh);
    int clash = 0;
    for (int i = 0; i < N; i++)
        if (i != 5 && ids[i] == fresh) clash = 1;
    check("fresh_distinct", !clash, 0);
    msq_rm(fresh);
    for (int i = 0; i < N; i++)
        if (i != 5) msq_rm(ids[i]);
    return finish();
}
