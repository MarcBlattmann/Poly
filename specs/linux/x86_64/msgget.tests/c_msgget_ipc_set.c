/* The permission check of a lookup uses the queue's current mode: after
 * IPC_SET changes the mode to 0400, asking for 0600 is EACCES and 0400
 * passes; after setting it back to 0600, 0600 passes again. IPC_SET
 * does not change the key or the id.
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
static i64 set_mode(i64 id, unsigned short mode) {
    struct k_msqid_ds ds;
    i64 r = msq_stat(id, &ds);
    if (r) return r;
    ds.mode = mode;
    return syscall3(SYS_msgctl, id, IPC_SET, &ds);
}

int test_main(void) {
    int key = own_key(14);
    i64 id = msgget(key, 0600 | IPC_CREAT);
    check("create_ok", id >= 0, id);
    i64 r = msgget(key, 0600);
    check("rw_ok", r == id, r);
    r = set_mode(id, 0400);
    check("set_0400", r == 0, r);
    r = msgget(key, 0600);
    check("after_set_rw_eacces", r == -EACCES, r);
    r = msgget(key, 0400);
    check("after_set_r_ok", r == id, r);
    r = msgget(key, 0600 | IPC_CREAT);
    check("after_set_creat_rw_eacces", r == -EACCES, r);
    r = set_mode(id, 0600);
    check("set_0600", r == 0, r);
    r = msgget(key, 0600);
    check("after_reset_rw_ok", r == id, r);
    struct k_msqid_ds ds;
    r = msq_stat(id, &ds);
    check("key_kept", r == 0 && ds.key == key, ds.key);
    msq_rm(id);
    return finish();
}
