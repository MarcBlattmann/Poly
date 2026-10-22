/* Who may do what, seen from a process that is not the queue's owner or
 * creator (a child of root that became uid/gid 65534): IPC_STAT and
 * MSG_STAT need read permission in its class (other, or group when its
 * gid is the queue's gid, in which case other bits do not help);
 * MSG_STAT_ANY needs none. IPC_SET and IPC_RMID by a non-owner are EPERM
 * whatever the mode (but IPC_SET reads its buffer first: NULL is
 * EFAULT). Once root gives the queue to that uid, it may set, read and
 * remove it. Root (CAP_IPC_OWNER, CAP_SYS_ADMIN, CAP_SYS_RESOURCE) reads
 * a mode-0 queue, sets and removes another user's queue, and may raise
 * qbytes above msgmnb (16384).
 * poly-diff: needs=root (changes uid and relies on root's capabilities)
 * poly-diff: syscalls=off (two processes; queue ids vary)
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

#define NOBODY 65534

/* Results the child leaves for the parent, by slot. */
enum {
    R_A_STAT, R_A_MSG_STAT, R_A_MSG_STAT_ANY, R_A_SET, R_A_SET_NULL, R_A_RMID,
    R_B_STAT, R_B_SET, R_B_RMID,
    R_D_GROUP_R, R_D_GROUP_DENIED_OTHER_R,
    R_C_CREATE,
    R_A_OWNER_STAT, R_A_OWNER_SET, R_A_OWNER_RMID,
    R_COUNT
};
static const char *const names[R_COUNT] = {
    "other_stat_0600_eacces", "other_msg_stat_0600_eacces", "other_msg_stat_any_ok", "other_set_eperm",
    "other_set_null_efault", "other_rmid_eperm",
    "other_stat_0644_ok", "other_set_0644_eperm", "other_rmid_0666_eperm",
    "group_r_ok", "group_class_other_r_denied",
    "child_creates_queue",
    "new_owner_stat_ok", "new_owner_set_ok", "new_owner_rmid_ok",
};

static i64 set_fields(i64 id, unsigned uid, unsigned gid, int mode, u64 qbytes) {
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    ds.uid = uid;
    ds.gid = gid;
    ds.mode = (unsigned short)mode;
    ds.qbytes = qbytes;
    return msgctl(id, IPC_SET, &ds);
}

int test_main(void) {
    volatile i64 *res = shared_page(4096);
    volatile unsigned *go = (volatile unsigned *)(res + 64);
    volatile unsigned *done = go + 1;
    volatile i64 *child_q = res + 80;
    check("shared_ok", res != 0, 0);

    i64 a = msgget(IPC_PRIVATE, 0600);
    i64 b = msgget(IPC_PRIVATE, 0666);
    i64 d = msgget(IPC_PRIVATE, 0600);
    check("create_ok", a >= 0 && b >= 0 && d >= 0, a);
    check("d_group_r", set_fields(d, 0, NOBODY, 0040, 16384) == 0, 0);

    i64 child = fork();
    if (child == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, NOBODY, NOBODY, NOBODY);
        if (syscall3(SYS_setresuid, NOBODY, NOBODY, NOBODY) != 0) {
            *child_q = -1;
            gate_open(go);
            exit_group(2);
        }
        struct k_msqid_ds ds;
        res[R_A_STAT] = msq_stat(a, &ds) == -EACCES;
        res[R_A_MSG_STAT] = msgctl(a & 0x7fff, MSG_STAT, &ds) == -EACCES;
        res[R_A_MSG_STAT_ANY] = msgctl(a & 0x7fff, MSG_STAT_ANY, &ds) == a && ds.mode == 0600;
        memset(&ds, 0, sizeof ds);
        ds.uid = NOBODY;
        ds.gid = NOBODY;
        ds.mode = 0666;
        res[R_A_SET] = msgctl(a, IPC_SET, &ds) == -EPERM;
        res[R_A_SET_NULL] = msgctl(a, IPC_SET, 0) == -EFAULT;
        res[R_A_RMID] = msq_rm(a) == -EPERM;
        res[R_B_STAT] = msq_stat(b, &ds) == 0 && ds.mode == 0666;
        res[R_B_SET] = msgctl(b, IPC_SET, &ds) == -EPERM;
        res[R_B_RMID] = msq_rm(b) == -EPERM;
        res[R_D_GROUP_R] = msq_stat(d, &ds) == 0;
        /* Root turns d's mode to 0004 next: group class, no r. */
        *child_q = msgget(IPC_PRIVATE, 0600);
        res[R_C_CREATE] = *child_q >= 0;
        gate_open(go);
        gate_wait(done);
        res[R_D_GROUP_DENIED_OTHER_R] = msq_stat(d, &ds) == -EACCES;
        res[R_A_OWNER_STAT] = msq_stat(a, &ds) == 0 && ds.uid == NOBODY;
        res[R_A_OWNER_SET] = set_fields(a, NOBODY, NOBODY, 0400, 100) == 0;
        res[R_A_OWNER_RMID] = msq_rm(a) == 0;
        exit_group(0);
    }
    gate_wait(go);
    i64 c = *child_q;
    if (c < 0) {
        check("child_became_nobody", 0, c);
        wait_exit(child);
        msq_rm(a);
        msq_rm(b);
        msq_rm(d);
        return finish();
    }
    check("d_other_r_only", set_fields(d, 0, NOBODY, 0004, 16384) == 0, 0);
    check("give_a_to_child", set_fields(a, NOBODY, NOBODY, 0600, 16384) == 0, 0);

    /* Root on the child's queue: mode 0, other owner. */
    struct k_msqid_ds ds;
    check("root_set_others_queue_mode_0", set_fields(c, NOBODY, NOBODY, 0, 16384) == 0, 0);
    i64 r = msq_stat(c, &ds);
    check("root_stat_mode_0", r == 0 && ds.uid == NOBODY && ds.cuid == NOBODY, r);
    r = msgctl(c & 0x7fff, MSG_STAT, &ds);
    check("root_msg_stat_mode_0", r == c, r);
    r = set_fields(c, NOBODY, NOBODY, 0, 20000);
    msq_stat(c, &ds);
    check("root_qbytes_above_msgmnb", r == 0 && ds.qbytes == 20000, r);
    r = set_fields(c, NOBODY, NOBODY, 0, 1000000);
    msq_stat(c, &ds);
    check("root_qbytes_1000000", r == 0 && ds.qbytes == 1000000, r);
    r = msq_rm(c);
    check("root_rm_others_queue", r == 0, r);

    gate_open(done);
    int st = wait_exit(child);
    check("child_exit_0", st == 0, st);
    for (int i = 0; i < R_COUNT; i++) check(names[i], res[i] == 1, res[i]);

    r = msq_stat(a, &ds);
    check("a_removed_by_new_owner", r == -EINVAL, r);
    check("rm_b", msq_rm(b) == 0, 0);
    check("rm_d", msq_rm(d) == 0, 0);
    return finish();
}
