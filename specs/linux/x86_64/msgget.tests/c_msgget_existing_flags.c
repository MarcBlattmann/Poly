/* IPC_CREAT on a used key is a lookup: the mode is not changed, and the
 * permission bits in msgflg are checked as for any lookup. On a mode 0
 * queue: flags 0 and IPC_CREAT alone find it; IPC_CREAT|0600 is EACCES;
 * IPC_CREAT|IPC_EXCL|0600 is EEXIST (reported before EACCES). An unused
 * key is ENOENT whatever mode bits are asked for. IPC_PRIVATE has no
 * permission check.
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
    int key = own_key(10);
    i64 id = msgget(key, IPC_CREAT);
    check("mode0_create_ok", id >= 0, id);
    i64 r = msgget(key, 0);
    check("mode0_lookup_ok", r == id, r);
    r = msgget(key, IPC_CREAT);
    check("mode0_creat_ok", r == id, r);
    r = msgget(key, IPC_CREAT | 0600);
    check("mode0_creat_rw_eacces", r == -EACCES, r);
    r = msgget(key, IPC_CREAT | IPC_EXCL | 0600);
    check("eexist_before_eacces", r == -EEXIST, r);
    r = msgget(key, IPC_CREAT | IPC_EXCL);
    check("mode0_excl_eexist", r == -EEXIST, r);
    msq_rm(id);

    key = own_key(11);
    id = msgget(key, IPC_CREAT | 0640);
    check("create_ok", id >= 0, id);
    r = msgget(key, IPC_CREAT | 0604);
    check("creat_other_mode_ok", r == id, r);
    struct k_msqid_ds ds;
    r = msq_stat(id, &ds);
    check("mode_unchanged", r == 0 && ds.mode == 0640, ds.mode);
    msq_rm(id);

    r = msgget(own_key(12), 0777);
    check("unused_any_mode_enoent", r == -ENOENT, r);
    r = msgget(own_key(12), 0);
    check("unused_mode0_enoent", r == -ENOENT, r);
    return finish();
}
