/* msgsz is a full 64-bit size: up to msgmax (8192, /proc/sys/kernel/msgmax)
 * is accepted; 8193, 2^32, 2^32+1 and (size_t)-1 are EINVAL and queue
 * nothing (the upper half is not ignored). msgsz 0 sends an empty text.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
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
int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);

    i64 r = send_fill(id, 1, 'x', 8193, IPC_NOWAIT);
    check("8193_einval", r == -EINVAL, r);
    r = send_fill(id, 1, 'x', 1UL << 32, IPC_NOWAIT);
    check("2p32_einval", r == -EINVAL, r);
    r = send_fill(id, 1, 'x', (1UL << 32) + 1, IPC_NOWAIT);
    check("2p32_plus_1_einval", r == -EINVAL, r);
    r = send_fill(id, 1, 'x', (u64)-1, IPC_NOWAIT);
    check("minus_1_einval", r == -EINVAL, r);
    r = send_fill(id, 1, 'x', 1UL << 63, IPC_NOWAIT);
    check("2p63_einval", r == -EINVAL, r);
    /* Without IPC_NOWAIT too: it never waits. */
    r = send_fill(id, 1, 'x', 8193, 0);
    check("8193_einval_blocking", r == -EINVAL, r);
    check("nothing_queued", msq_qnum(id) == 0, msq_qnum(id));

    r = send_fill(id, 1, 'x', 8192, IPC_NOWAIT);
    check("8192_ok", r == 0, r);
    r = send_fill(id, 1, 'x', 0, IPC_NOWAIT);
    check("0_ok", r == 0, r);
    r = send_fill(id, 1, 'x', 1, IPC_NOWAIT);
    check("1_ok", r == 0, r);
    check("qnum_3", msq_qnum(id) == 3, msq_qnum(id));
    check("cbytes_8193", msq_cbytes(id) == 8193, msq_cbytes(id));

    msq_rm(id);
    return finish();
}
