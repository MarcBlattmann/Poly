/* IPC_SET of qbytes: only the low 32 bits of the 64-bit field count.
 * Any value up to msgmnb (16384) is accepted, 0 included; a larger one
 * needs CAP_SYS_RESOURCE, without it the call fails with EPERM and changes
 * nothing at all (mode, uid and gid in the same buffer are not applied
 * either). The EPERM for qbytes is reported before an invalid uid's
 * EINVAL. Root (with the capability) may set 16385 and more.
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

static i64 id;
static struct k_msqid_ds ds;

static i64 set_qbytes(u64 q) {
    msq_stat(id, &ds);
    ds.qbytes = q;
    return msgctl(id, IPC_SET, &ds);
}
static u64 qbytes(void) {
    msq_stat(id, &ds);
    return ds.qbytes;
}

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    id = msgget(IPC_PRIVATE, 0600);
    check("create_ok", id >= 0, id);

    check("set_16383", set_qbytes(16383) == 0 && qbytes() == 16383, (i64)qbytes());
    check("set_16384", set_qbytes(16384) == 0 && qbytes() == 16384, (i64)qbytes());
    check("set_1", set_qbytes(1) == 0 && qbytes() == 1, (i64)qbytes());
    check("set_0", set_qbytes(0) == 0 && qbytes() == 0, (i64)qbytes());
    i64 r = set_qbytes(1UL << 32);
    check("set_2pow32_is_0", r == 0 && qbytes() == 0, (i64)qbytes());
    set_qbytes(100);
    r = set_qbytes((1UL << 32) | 5);
    check("set_2pow32_plus5_is_5", r == 0 && qbytes() == 5, (i64)qbytes());
    r = set_qbytes(1UL << 63);
    check("set_2pow63_is_0", r == 0 && qbytes() == 0, (i64)qbytes());
    r = set_qbytes((7UL << 40) | 16384);
    check("set_high_bits_16384", r == 0 && qbytes() == 16384, (i64)qbytes());

    r = set_qbytes(16385);
    if (root) {
        check("root_set_16385", r == 0 && qbytes() == 16385, (i64)qbytes());
        set_qbytes(16384);
    } else {
        check("set_16385_eperm", r == -EPERM, r);
        check("set_16385_kept", qbytes() == 16384, (i64)qbytes());
        r = set_qbytes((1UL << 32) | 16385);
        check("set_high_bits_16385_eperm", r == -EPERM, r);
        r = set_qbytes(0x80000000UL);
        check("set_2pow31_eperm", r == -EPERM, r);
        r = set_qbytes(0xffffffffUL);
        check("set_u32max_eperm", r == -EPERM, r);
        r = set_qbytes(~0UL);
        check("set_u64max_eperm", r == -EPERM, r);
        check("qbytes_still_16384", qbytes() == 16384, (i64)qbytes());

        /* A refused call applies nothing of the buffer. */
        msq_stat(id, &ds);
        ds.qbytes = 20000;
        ds.mode = 0444;
        ds.gid = 4343;
        r = msgctl(id, IPC_SET, &ds);
        check("mixed_eperm", r == -EPERM, r);
        msq_stat(id, &ds);
        check("mixed_mode_kept", ds.mode == 0600, ds.mode);
        check("mixed_gid_kept", ds.gid == syscall0(SYS_getegid), ds.gid);

        /* EPERM (qbytes) before EINVAL (uid -1). */
        ds.qbytes = 20000;
        ds.uid = 0xffffffffU;
        r = msgctl(id, IPC_SET, &ds);
        check("eperm_before_bad_uid", r == -EPERM, r);
    }

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
