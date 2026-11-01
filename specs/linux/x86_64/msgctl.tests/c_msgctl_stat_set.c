/* IPC_STAT fills a struct msqid64_ds; IPC_SET takes uid, gid, the low 9
 * bits of mode and msg_qbytes from one (other fields ignored: cuid,
 * qnum, cbytes, times and pids are kept), sets msg_ctime. Raising
 * msg_qbytes above msgmnb (16384) needs CAP_SYS_RESOURCE (EPERM
 * otherwise, checked before the uid/gid validity); 16384 itself and any
 * lower value (also below the bytes queued) are fine. uid or gid -1 is
 * EINVAL. NULL buffers are EFAULT.
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
    unsigned uid = (unsigned)syscall0(SYS_geteuid), gid = (unsigned)syscall0(SYS_getegid);
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    send_text(id, 1, "abcd", 4, 0);
    struct k_msqid_ds ds, set;
    msg_stat(id, &ds);
    i64 stime = ds.stime;

    set = ds;
    set.mode = 0177640;
    set.cuid = 4242;
    set.qnum = 50;
    set.cbytes = 50;
    set.stime = 1;
    set.lspid = 1;
    set.qbytes = 2;
    set.ctime = 1;
    i64 r = msgctl(id, IPC_SET, &set);
    check("set_ok", r == 0, r);
    msg_stat(id, &ds);
    check("mode_low_9_bits", ds.mode == 0640, ds.mode);
    check("qbytes_2_below_cbytes", ds.qbytes == 2, (i64)ds.qbytes);
    check("cuid_kept", ds.cuid == uid, ds.cuid);
    check("qnum_cbytes_kept", ds.qnum == 1 && ds.cbytes == 4, (i64)ds.qnum);
    check("stime_lspid_kept", ds.stime == stime && ds.lspid == getpid(), ds.lspid);
    check("ctime_updated", ds.ctime > 1, ds.ctime);
    static struct k_msg m;
    r = msgrcv(id, &m, 100, 0, IPC_NOWAIT);
    check("message_survives", r == 4, r);

    set = ds;
    set.qbytes = 16384;
    r = msgctl(id, IPC_SET, &set);
    check("qbytes_16384_ok", r == 0, r);
    set.qbytes = 16385;
    r = msgctl(id, IPC_SET, &set);
    check("qbytes_16385", r == (root ? 0 : -EPERM), r);
    set.qbytes = 100000;
    set.uid = 0xffffffffu;
    r = msgctl(id, IPC_SET, &set);
    check("qbytes_eperm_before_uid_einval", r == (root ? -EINVAL : -EPERM), r);
    set.qbytes = 100;
    r = msgctl(id, IPC_SET, &set);
    check("uid_minus1_einval", r == -EINVAL, r);
    set.uid = uid;
    set.gid = 0xffffffffu;
    r = msgctl(id, IPC_SET, &set);
    check("gid_minus1_einval", r == -EINVAL, r);
    msg_stat(id, &ds);
    check("einval_changed_nothing", ds.qbytes == (root ? 16385u : 16384u) && ds.gid == gid, (i64)ds.qbytes);

    set.gid = gid;
    set.uid = 777;
    set.mode = 0600;
    r = msgctl(id, IPC_SET, &set);
    check("give_away_ok", r == 0, r);
    msg_stat(id, &ds);
    check("given_uid_creator_still_stats", ds.uid == 777 && ds.cuid == uid, ds.uid);
    r = msgctl(id, IPC_SET, 0);
    check("set_null_efault", r == -EFAULT, r);
    r = msgctl(id, IPC_STAT, 0);
    check("stat_null_efault", r == -EFAULT, r);
    r = msgctl(id, IPC_RMID, (void *)1);
    check("rmid_ignores_buffer", r == 0, r);
    return finish();
}
