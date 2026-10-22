/* Argument checks. The valid commands are IPC_RMID 0, IPC_SET 1,
 * IPC_STAT 2, IPC_INFO 3, MSG_STAT 11, MSG_INFO 12 and MSG_STAT_ANY 13;
 * every other value (negative, 4-10, 14 and up, and IPC_64 0x100 or'ed
 * into a valid one) is EINVAL. A negative id is EINVAL for every
 * command, the info ones included, and before anything else (a NULL
 * buffer too). Only the low 32 bits of the id and of the command count.
 * An id that was never handed out, or whose index is in use but whose
 * sequence number differs, is EINVAL.
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
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("create_ok", id >= 0, id);
    struct k_msqid_ds ds;
    char big[256];

    static const int bad_cmds[] = { -1, 4, 5, 6, 7, 8, 9, 10, 14, 15, 16, 17, 19, 20, 100, 0x7fffffff };
    int bad_ok = 1;
    for (unsigned i = 0; i < sizeof bad_cmds / sizeof bad_cmds[0]; i++) {
        memset(big, 0, sizeof big);
        if (msgctl(id, bad_cmds[i], big) != -EINVAL) bad_ok = 0;
    }
    check("unknown_cmds_einval", bad_ok, 0);
    i64 r = msgctl(id, IPC_STAT | IPC_64, &ds);
    check("stat_ipc64_einval", r == -EINVAL, r);
    r = msgctl(id, IPC_RMID | IPC_64, 0);
    check("rmid_ipc64_einval", r == -EINVAL, r);
    r = msgctl(0, IPC_INFO | IPC_64, big);
    check("info_ipc64_einval", r == -EINVAL, r);
    r = msq_stat(id, &ds);
    check("queue_still_there", r == 0, r);

    r = msgctl(-1, IPC_STAT, 0);
    check("neg_id_stat_einval", r == -EINVAL, r);
    r = msgctl(-1, IPC_SET, 0);
    check("neg_id_set_einval", r == -EINVAL, r);
    r = msgctl(-1, IPC_RMID, 0);
    check("neg_id_rmid_einval", r == -EINVAL, r);
    r = msgctl(-1, IPC_INFO, big);
    check("neg_id_ipc_info_einval", r == -EINVAL, r);
    r = msgctl(-5, MSG_INFO, big);
    check("neg_id_msg_info_einval", r == -EINVAL, r);
    r = msgctl(-1, MSG_STAT, &ds);
    check("neg_id_msg_stat_einval", r == -EINVAL, r);
    r = msgctl((i64)0x80000000, IPC_STAT, &ds);
    check("id_int_min_einval", r == -EINVAL, r);
    r = msgctl(0xffffffffL, IPC_STAT, &ds);
    check("id_u32max_is_negative", r == -EINVAL, r);
    r = msgctl(-1, 77, 0);
    check("neg_id_bad_cmd_einval", r == -EINVAL, r);

    memset(&ds, 0, sizeof ds);
    r = msgctl(id | (1L << 32), IPC_STAT, &ds);
    check("id_high_bits_ignored", r == 0 && ds.qbytes == 16384, r);
    memset(&ds, 0, sizeof ds);
    r = msgctl(id, IPC_STAT | (1L << 32), &ds);
    check("cmd_high_bits_ignored", r == 0 && ds.qbytes == 16384, r);
    r = msgctl(id, IPC_STAT | (0xffffffffL << 32), &ds);
    check("cmd_all_high_bits_ignored", r == 0, r);

    r = msgctl(id + 32768, IPC_STAT, &ds);
    check("other_seq_einval", r == -EINVAL, r);
    r = msgctl(id + 32768, IPC_RMID, 0);
    check("other_seq_rmid_einval", r == -EINVAL, r);
    r = msgctl(0x7ffffff0, IPC_STAT, &ds);
    check("unknown_id_einval", r == -EINVAL, r);
    r = msgctl(0x7ffffff0, IPC_RMID, 0);
    check("unknown_id_rmid_einval", r == -EINVAL, r);

    r = msgctl(id | (1L << 32), IPC_RMID | (1L << 32), 0);
    check("rmid_high_bits_ok", r == 0, r);
    r = msq_stat(id, &ds);
    check("gone", r == -EINVAL, r);
    return finish();
}
