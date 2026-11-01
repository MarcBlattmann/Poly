/* msgsnd appends a message (a long mtype >= 1 and msgsz bytes of text)
 * to the queue and returns 0: msg_qnum +1, msg_cbytes +msgsz, msg_lspid
 * = caller, msg_stime = now; msg_rtime/lrpid unchanged. Messages keep
 * their order. msgsz may be 0 and at most msgmax (8192); 8193 or a
 * size_t with the top bit set is EINVAL. mtype 0 or negative is EINVAL.
 * Nothing is queued by a failed call. msgflg bits other than IPC_NOWAIT
 * are ignored.
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
    i64 t0 = now_sec();
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    i64 r = send_text(id, 5, "hello", 5, 0);
    check("send_ok", r == 0, r);
    struct k_msqid_ds ds;
    msg_stat(id, &ds);
    check("qnum_1", ds.qnum == 1, (i64)ds.qnum);
    check("cbytes_5", ds.cbytes == 5, (i64)ds.cbytes);
    check("lspid_caller", ds.lspid == getpid(), 0);
    check("stime_now", ds.stime >= t0 && ds.stime <= now_sec(), ds.stime - t0);
    check("rtime_lrpid_0", ds.rtime == 0 && ds.lrpid == 0, ds.lrpid);

    r = send_text(id, 1, 0, 0, 0);
    check("zero_length_ok", r == 0, r);
    r = send_text(id, 0x7fffffffffffffffL, 0, 8192, 0);
    check("msgmax_ok", r == 0, r);
    r = send_text(id, 3, "xyz", 3, MSG_NOERROR | 0x100000);
    check("extra_flags_ignored", r == 0, r);
    msg_stat(id, &ds);
    check("qnum_4", ds.qnum == 4, (i64)ds.qnum);
    check("cbytes_8200", ds.cbytes == 8200, (i64)ds.cbytes);

    r = send_text(id, 1, 0, 8193, 0);
    check("8193_einval", r == -EINVAL, r);
    static struct k_msg m;
    m.mtype = 1;
    r = msgsnd(id, &m, 0x8000000000000000UL, 0);
    check("huge_size_einval", r == -EINVAL, r);
    r = msgsnd(id, &m, (1UL << 32) | 1, 0);
    check("size_is_64_bit_einval", r == -EINVAL, r);
    r = send_text(id, 0, "a", 1, 0);
    check("type_0_einval", r == -EINVAL, r);
    r = send_text(id, -1, "a", 1, 0);
    check("type_negative_einval", r == -EINVAL, r);
    r = send_text(id, (i64)0x8000000000000000UL, "a", 1, 0);
    check("type_min_einval", r == -EINVAL, r);
    msg_stat(id, &ds);
    check("failed_sends_queue_nothing", ds.qnum == 4 && ds.cbytes == 8200, (i64)ds.qnum);

    /* Order and contents. */
    r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("recv1", r == 5 && m.mtype == 5 && mem_eq(m.mtext, "hello", 5), r);
    r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("recv2_zero_length", r == 0 && m.mtype == 1, r);
    r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("recv3_max", r == 8192 && m.mtype == 0x7fffffffffffffffL && m.mtext[8191] == 'x', r);
    r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("recv4", r == 3 && m.mtype == 3, r);
    r = msgsnd(id | (1L << 32), &m, 1, (1L << 32));
    check("id_and_flag_high_bits_ignored", r == 0, r);
    msg_rm(id);
    return finish();
}
