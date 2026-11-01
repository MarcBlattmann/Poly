/* Sending needs write permission for the caller's class (owner here):
 * mode 0400 is EACCES for the owner, also when the group/other bits
 * have w (0422); x does not help. Root (CAP_IPC_OWNER) may always send.
 * EACCES comes before the room check (a full queue without w is
 * EACCES, not EAGAIN), after EINVAL for the size and the id.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
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
    int root = syscall0(SYS_geteuid) == 0;
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    send_text(id, 1, 0, 8192, 0);
    send_text(id, 1, 0, 8192, 0);
    struct k_msqid_ds ds;
    msg_stat(id, &ds);
    ds.mode = 0422;
    check("set_0422", msgctl(id, IPC_SET, &ds) == 0, 0);
    i64 r = send_text(id, 1, 0, 1, IPC_NOWAIT);
    check("full_no_w", r == (root ? -EAGAIN : -EACCES), r);
    r = send_text(id, 1, 0, 8193, IPC_NOWAIT);
    check("size_einval_before_access", r == -EINVAL, r);
    msg_stat(id, &ds);
    ds.mode = 0500;
    msgctl(id, IPC_SET, &ds);
    static struct k_msg m;
    msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    r = send_text(id, 1, 0, 1, IPC_NOWAIT);
    check("owner_rx_no_w", r == (root ? 0 : -EACCES), r);
    msg_stat(id, &ds);
    ds.mode = 0200;
    msgctl(id, IPC_SET, &ds);
    r = send_text(id, 1, 0, 1, IPC_NOWAIT);
    check("owner_w_ok", r == 0, r);
    msg_rm(id);
    return finish();
}
