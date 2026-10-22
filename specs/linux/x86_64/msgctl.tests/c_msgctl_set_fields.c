/* IPC_SET takes uid, gid, the 0777 bits of mode and qbytes from the
 * buffer and nothing else: key, cuid, cgid, seq, the times, qnum, cbytes,
 * lspid and lrpid in the buffer are ignored. It returns 0 and sets ctime
 * to the current time (even when nothing changes). The owner may give the
 * queue to any uid and gid; the creator (cuid) still counts as owner
 * afterwards and may take it back.
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
    int key = own_key(2);
    i64 id = msgget(key, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    check("send_ok", msq_send(id, 7, 5, 0) == 0, 0);

    struct k_msqid_ds ds, before;
    msq_stat(id, &before);
    /* Sleep past the next second so a new ctime shows. */
    sleep_ms(1100);

    ds = before;
    ds.key = 1234;
    ds.cuid = 4242;
    ds.cgid = 4343;
    ds.mode = 0177640;
    ds.seq = 99;
    ds.stime = 5;
    ds.rtime = 6;
    ds.ctime = 7;
    ds.cbytes = 1000;
    ds.qnum = 77;
    ds.qbytes = 1000;
    ds.lspid = 1;
    ds.lrpid = 2;
    i64 t0 = now_sec();
    i64 r = msgctl(id, IPC_SET, &ds);
    i64 t1 = now_sec();
    check("set_returns_0", r == 0, r);
    r = msq_stat(id, &ds);
    check("stat_ok", r == 0, r);
    check("mode_low9_bits", ds.mode == 0640, ds.mode);
    check("qbytes_set", ds.qbytes == 1000, (i64)ds.qbytes);
    check("key_kept", ds.key == key, ds.key);
    check("cuid_kept", ds.cuid == euid, ds.cuid);
    check("cgid_kept", ds.cgid == egid, ds.cgid);
    check("seq_kept", ds.seq == before.seq, ds.seq);
    check("stime_kept", ds.stime == before.stime, ds.stime - before.stime);
    check("rtime_kept", ds.rtime == 0, ds.rtime);
    check("cbytes_kept", ds.cbytes == 5, (i64)ds.cbytes);
    check("qnum_kept", ds.qnum == 1, (i64)ds.qnum);
    check("lspid_kept", ds.lspid == before.lspid, ds.lspid);
    check("lrpid_kept", ds.lrpid == 0, ds.lrpid);
    check("ctime_now", ds.ctime >= t0 && ds.ctime <= t1, ds.ctime - t0);
    check("ctime_moved", ds.ctime > before.ctime, ds.ctime - before.ctime);

    /* Same values again: still a change of ctime. */
    i64 c1 = ds.ctime;
    sleep_ms(1100);
    r = msgctl(id, IPC_SET, &ds);
    check("set_same_ok", r == 0, r);
    msq_stat(id, &ds);
    check("ctime_moved_again", ds.ctime > c1, ds.ctime - c1);

    /* Give the queue away; the creator still owns it. */
    ds.uid = 4242;
    ds.gid = 4343;
    ds.mode = 0600;
    r = msgctl(id, IPC_SET, &ds);
    check("give_away_ok", r == 0, r);
    r = msq_stat(id, &ds);
    check("creator_may_stat", r == 0, r);
    check("uid_given", ds.uid == 4242, ds.uid);
    check("gid_given", ds.gid == 4343, ds.gid);
    check("cuid_still_me", ds.cuid == euid, ds.cuid);
    ds.uid = (unsigned)euid;
    ds.gid = (unsigned)egid;
    ds.mode = 0;
    r = msgctl(id, IPC_SET, &ds);
    check("creator_takes_back", r == 0, r);
    ds.mode = 0644;
    r = msgctl(id, IPC_SET, &ds);
    check("owner_set_with_mode_0", r == 0, r);
    r = msq_stat(id, &ds);
    check("uid_back", r == 0 && ds.uid == euid, ds.uid);
    check("gid_back", ds.gid == egid, ds.gid);
    check("mode_back", ds.mode == 0644, ds.mode);

    /* The queue's messages are untouched by all of it. */
    r = msq_recv(id, IPC_NOWAIT);
    check("message_kept", r == 5, r);
    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
