/* IPC_RMID removes the queue at once and returns 0; the buffer argument
 * is ignored (any pointer). Its messages are gone, and the id is dead:
 * IPC_STAT, IPC_SET, IPC_RMID, msgsnd and msgrcv on it are EINVAL (not
 * EIDRM). Its key is free again: msgget without IPC_CREAT is ENOENT and
 * IPC_CREAT|IPC_EXCL makes a new queue with another id, empty.
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
    int key = own_key(5);
    i64 id = msgget(key, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    check("send1", msq_send(id, 1, 8, 0) == 0, 0);
    check("send2", msq_send(id, 2, 8, 0) == 0, 0);

    i64 r = msgctl(id, IPC_RMID, (void *)1);
    check("rmid_junk_pointer_ok", r == 0, r);

    struct k_msqid_ds ds;
    r = msq_stat(id, &ds);
    check("stat_removed_einval", r == -EINVAL, r);
    r = msgctl(id, IPC_SET, &ds);
    check("set_removed_einval", r == -EINVAL, r);
    r = msq_rm(id);
    check("rmid_twice_einval", r == -EINVAL, r);
    r = msq_send(id, 1, 1, IPC_NOWAIT);
    check("send_removed_einval", r == -EINVAL, r);
    r = msq_recv(id, IPC_NOWAIT);
    check("recv_removed_einval", r == -EINVAL, r);

    r = msgget(key, 0600);
    check("key_lookup_enoent", r == -ENOENT, r);
    i64 id2 = msgget(key, 0600 | IPC_CREAT | IPC_EXCL);
    check("key_recreate_ok", id2 >= 0, id2);
    check("new_id_differs", id2 != id, id2 - id);
    r = msq_stat(id2, &ds);
    check("new_queue_empty", r == 0 && ds.qnum == 0 && ds.cbytes == 0, (i64)ds.qnum);
    r = msq_stat(id, &ds);
    check("old_id_still_dead", r == -EINVAL, r);

    r = msgctl(id2, IPC_RMID, 0);
    check("rmid_null_ok", r == 0, r);
    return finish();
}
