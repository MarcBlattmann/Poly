/* Only the low 32 bits of key and msgflg count: key | 1<<32 finds the
 * queue of key, and msgflg | 1<<32 is msgflg. Keys are signed ints:
 * a negative key such as -7 works and IPC_STAT shows it as -7.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
/* linux/ipc.h, linux/msg.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
/* asm-generic/errno.h */
#define ENOMSG 42
#define EIDRM 43

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

/* struct msgbuf (linux/msg.h) with a small text. */
struct k_msg {
    i64 mtype;
    char mtext[16];
};

static __attribute__((unused)) i64 msgget(i64 key, i64 flg) { return syscall2(SYS_msgget, key, flg); }
static __attribute__((unused)) i64 msq_rm(i64 id) { return syscall3(SYS_msgctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 msq_stat(i64 id, struct k_msqid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_msgctl, id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 msq_send(i64 id, i64 type, const char *text, u64 len) {
    struct k_msg m;
    memset(&m, 0, sizeof m);
    m.mtype = type;
    memcpy(m.mtext, text, len);
    return syscall4(SYS_msgsnd, id, &m, len, IPC_NOWAIT);
}
static __attribute__((unused)) i64 msq_recv(i64 id, struct k_msg *m) {
    memset(m, 0, sizeof *m);
    return syscall5(SYS_msgrcv, id, m, sizeof m->mtext, 0, IPC_NOWAIT);
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x40000000 | (tag << 22) | (int)getpid()); }
int test_main(void) {
    int key = own_key(5);
    i64 id = msgget(key, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    i64 r = msgget((1L << 32) | (u64)(unsigned)key, 0);
    check("key_upper_ignored", r == id, r);
    r = msgget((i64)0xffffffff00000000UL | (u64)(unsigned)key, 0);
    check("key_upper_ones_ignored", r == id, r);
    r = msgget(key, (1L << 32) | IPC_CREAT | IPC_EXCL | 0600);
    check("flag_upper_ignored", r == -EEXIST, r);
    r = msgget(own_key(6), (1L << 32) | 0600);
    check("flag_upper_no_creat", r == -ENOENT, r);
    msq_rm(id);

    /* A negative key: -(pid) - 7, unique per run. */
    int neg = -7 - (int)getpid();
    id = msgget(neg, 0600 | IPC_CREAT | IPC_EXCL);
    check("negative_create_ok", id >= 0, id);
    struct k_msqid_ds ds;
    r = msq_stat(id, &ds);
    check("negative_key_kept", r == 0 && ds.key == neg, ds.key - neg);
    r = msgget((u64)(unsigned)neg, 0);
    check("negative_zero_extended", r == id, r);
    r = msgget(neg, 0);
    check("negative_sign_extended", r == id, r);
    msq_rm(id);
    return finish();
}
