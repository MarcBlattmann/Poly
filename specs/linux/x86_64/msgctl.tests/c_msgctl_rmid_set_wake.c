/* IPC_RMID removes a queue at once, with its messages: the key is free
 * again, the id is EINVAL, sleeping receivers and senders return EIDRM.
 * IPC_SET that raises msg_qbytes lets a sender sleeping on a full queue
 * continue.
 * poly-diff: syscalls=off (several processes, timing-dependent)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define E2BIG 7
#define ENOSPC 28
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
#define MSG_STAT 11
#define MSG_INFO 12
#define MSG_STAT_ANY 13
#define MSG_NOERROR 010000
#define MSG_EXCEPT 020000
#define MSG_COPY 040000

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

/* struct msgbuf (linux/msg.h): a long type, then the text. */
struct k_msg {
    i64 mtype;
    char mtext[8200];
};

static __attribute__((unused)) i64 msgget(i64 key, i64 flg) { return syscall2(SYS_msgget, key, flg); }
static __attribute__((unused)) i64 msgsnd(i64 id, const void *msgp, u64 sz, i64 flg) {
    return syscall4(SYS_msgsnd, id, msgp, sz, flg);
}
static __attribute__((unused)) i64 msgrcv(i64 id, void *msgp, u64 sz, i64 typ, i64 flg) {
    return syscall5(SYS_msgrcv, id, msgp, sz, typ, flg);
}
static __attribute__((unused)) i64 msgctl(i64 id, i64 cmd, void *buf) { return syscall3(SYS_msgctl, id, cmd, buf); }
static __attribute__((unused)) i64 msg_rm(i64 id) { return msgctl(id, IPC_RMID, 0); }
static __attribute__((unused)) i64 msg_stat(i64 id, struct k_msqid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return msgctl(id, IPC_STAT, ds);
}
/* Send `len` bytes of `text` (or of 'x' when text is 0) with type `type`. */
static __attribute__((unused)) i64 send_text(i64 id, i64 type, const char *text, u64 len, i64 flg) {
    static struct k_msg m;
    m.mtype = type;
    for (u64 i = 0; i < len; i++) m.mtext[i] = text ? text[i] : 'x';
    return msgsnd(id, &m, len, flg);
}
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x40000000 | (tag << 22) | (int)getpid()); }

int test_main(void) {
    static struct k_msg m;
    struct k_msqid_ds ds;
    int key = own_key(1);
    i64 id = msgget(key, 0600 | IPC_CREAT | IPC_EXCL);
    check("msgget_ok", id >= 0, id);
    send_text(id, 1, "x", 1, 0);
    i64 r = msg_rm(id);
    check("rm_with_messages_ok", r == 0, r);
    check("key_free", msgget(key, 0) == -ENOENT, 0);
    r = msgrcv(id, &m, 10, 0, IPC_NOWAIT);
    check("id_einval", r == -EINVAL, r);

    id = msgget(IPC_PRIVATE, 0600);
    i64 rx = fork();
    if (rx == 0) exit_group(msgrcv(id, &m, 10, 5, 0) == -EIDRM ? 0 : 1);
    send_text(id, 1, 0, 8192, 0);
    send_text(id, 1, 0, 8192, 0);
    i64 tx = fork();
    if (tx == 0) exit_group(send_text(id, 1, 0, 100, 0) == -EIDRM ? 0 : 1);
    sleep_ms(300);
    r = msg_rm(id);
    check("rm_ok", r == 0, r);
    check("receiver_eidrm", wait_exit(rx) == 0, 0);
    check("sender_eidrm", wait_exit(tx) == 0, 0);

    id = msgget(IPC_PRIVATE, 0600);
    msg_stat(id, &ds);
    ds.qbytes = 10;
    msgctl(id, IPC_SET, &ds);
    send_text(id, 1, 0, 10, 0);
    tx = fork();
    if (tx == 0) exit_group(send_text(id, 2, 0, 5, 0) == 0 ? 0 : 1);
    sleep_ms(300);
    msg_stat(id, &ds);
    check("sender_waits", ds.qnum == 1, (i64)ds.qnum);
    ds.qbytes = 15;
    r = msgctl(id, IPC_SET, &ds);
    check("raise_qbytes_ok", r == 0, r);
    check("sender_continues", wait_exit(tx) == 0, 0);
    msg_stat(id, &ds);
    check("both_queued", ds.qnum == 2 && ds.cbytes == 15, (i64)ds.cbytes);
    msg_rm(id);
    return finish();
}
