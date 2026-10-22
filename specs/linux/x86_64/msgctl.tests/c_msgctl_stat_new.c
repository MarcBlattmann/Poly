/* IPC_STAT of a new queue: returns 0 and fills all 120 bytes of struct
 * msqid64_ds: key as given (0 for IPC_PRIVATE), uid/cuid and gid/cgid the
 * caller's effective ids, mode the 0777 bits of msgflg, seq the queue id
 * divided by 32768, every padding and unused field 0, no messages, qbytes
 * 16384 (msgmnb), lspid/lrpid/stime/rtime 0, ctime the creation time.
 * IPC_STAT itself changes nothing (a second call gives the same bytes).
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
    i64 euid = syscall0(SYS_geteuid), egid = syscall0(SYS_getegid);
    i64 t0 = now_sec();
    int key = own_key(1);
    i64 id = msgget(key, 0754 | IPC_CREAT | IPC_EXCL);
    i64 t1 = now_sec();
    check("create_ok", id >= 0, id);

    struct k_msqid_ds ds;
    memset(&ds, 0xa5, sizeof ds);
    i64 r = msgctl(id, IPC_STAT, &ds);
    check("stat_returns_0", r == 0, r);
    check("key", ds.key == key, ds.key);
    check("uid", ds.uid == euid, ds.uid);
    check("gid", ds.gid == egid, ds.gid);
    check("cuid", ds.cuid == euid, ds.cuid);
    check("cgid", ds.cgid == egid, ds.cgid);
    check("mode", ds.mode == 0754, ds.mode);
    check("seq_is_id_div_32768", ds.seq == (unsigned short)(id >> 15), ds.seq);
    check("pad1_zero", ds.pad1 == 0, ds.pad1);
    check("pad2_zero", ds.pad2 == 0, ds.pad2);
    check("pad3_zero", ds.pad3 == 0, ds.pad3);
    check("unused1_zero", ds.unused1 == 0, (i64)ds.unused1);
    check("unused2_zero", ds.unused2 == 0, (i64)ds.unused2);
    check("unused4_zero", ds.unused4 == 0, (i64)ds.unused4);
    check("unused5_zero", ds.unused5 == 0, (i64)ds.unused5);
    check("stime_zero", ds.stime == 0, ds.stime);
    check("rtime_zero", ds.rtime == 0, ds.rtime);
    check("ctime_now", ds.ctime >= t0 && ds.ctime <= t1, ds.ctime - t0);
    check("cbytes_zero", ds.cbytes == 0, (i64)ds.cbytes);
    check("qnum_zero", ds.qnum == 0, (i64)ds.qnum);
    check("qbytes_msgmnb", ds.qbytes == 16384, (i64)ds.qbytes);
    check("lspid_zero", ds.lspid == 0, ds.lspid);
    check("lrpid_zero", ds.lrpid == 0, ds.lrpid);

    struct k_msqid_ds again;
    memset(&again, 0x5a, sizeof again);
    r = msgctl(id, IPC_STAT, &again);
    check("stat_again_ok", r == 0, r);
    check("stat_again_same_bytes", mem_eq(&ds, &again, sizeof ds), 0);

    i64 pid = msgget(IPC_PRIVATE, 0600);
    r = msq_stat(pid, &ds);
    check("private_stat_ok", r == 0, r);
    check("private_key_zero", ds.key == 0, ds.key);
    check("private_mode", ds.mode == 0600, ds.mode);

    check("rm_ok", msq_rm(id) == 0, 0);
    check("rm_private_ok", msq_rm(pid) == 0, 0);
    return finish();
}
