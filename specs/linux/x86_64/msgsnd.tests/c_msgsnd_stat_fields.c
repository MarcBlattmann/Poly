/* What a send changes in IPC_STAT: lspid becomes the caller's pid and
 * stime the current time; qnum and cbytes grow. ctime, lrpid and rtime
 * are not touched. A failed send (EAGAIN on a full queue, EINVAL,
 * EFAULT) changes none of these.
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
static i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    i64 ctime0 = ds.ctime;
    check("new_lspid_0", ds.lspid == 0, ds.lspid);
    check("new_stime_0", ds.stime == 0, ds.stime);

    /* Let the clock move so a changed ctime would show. */
    sleep_ms(1100);
    i64 t0 = now_sec();
    i64 r = send_fill(id, 1, 'a', 3, 0);
    i64 t1 = now_sec();
    check("send_ok", r == 0, r);
    msq_stat(id, &ds);
    check("lspid_caller", ds.lspid == getpid(), ds.lspid);
    check("stime_now", ds.stime >= t0 && ds.stime <= t1, ds.stime - t0);
    check("ctime_unchanged", ds.ctime == ctime0, ds.ctime - ctime0);
    check("lrpid_0", ds.lrpid == 0, ds.lrpid);
    check("rtime_0", ds.rtime == 0, ds.rtime);
    check("qnum_1", ds.qnum == 1, (i64)ds.qnum);
    check("cbytes_3", ds.cbytes == 3, (i64)ds.cbytes);

    /* Failed sends change nothing: lower qbytes so 3 more bytes do not fit. */
    i64 stime1 = ds.stime;
    msq_set(id, 4, 0600);
    msq_stat(id, &ds);
    i64 ctime1 = ds.ctime;
    sleep_ms(1100);
    r = send_fill(id, 1, 'b', 3, IPC_NOWAIT);
    check("full_eagain", r == -EAGAIN, r);
    r = send_fill(id, 0, 'b', 1, IPC_NOWAIT);
    check("type0_einval", r == -EINVAL, r);
    r = msgsnd(id, 0, 1, IPC_NOWAIT);
    check("null_efault", r == -EFAULT, r);
    msq_stat(id, &ds);
    check("fail_qnum", ds.qnum == 1, (i64)ds.qnum);
    check("fail_cbytes", ds.cbytes == 3, (i64)ds.cbytes);
    check("fail_stime", ds.stime == stime1, ds.stime - stime1);
    check("fail_ctime", ds.ctime == ctime1, ds.ctime - ctime1);
    check("fail_lspid", ds.lspid == getpid(), ds.lspid);

    /* A child's send makes it the lspid. */
    i64 pid = fork();
    if (pid == 0) {
        static struct k_msg m;
        msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
        exit_group(send_fill(id, 2, 'c', 1, IPC_NOWAIT) == 0 ? 0 : 1);
    }
    check("child_ok", wait_exit(pid) == 0, 0);
    msq_stat(id, &ds);
    check("lspid_child", ds.lspid == pid, ds.lspid);

    msq_rm(id);
    return finish();
}
