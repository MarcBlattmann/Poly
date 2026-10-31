/* Finding a used key checks the access asked for in the low 9 bits of
 * msgflg: r, w and x of all three classes merged, each merged bit must
 * be granted by the queue's mode for the caller's class (owner here).
 * Missing: EACCES. Asking for 0 always works; root (CAP_IPC_OWNER)
 * passes every check. A mode 0 queue can be made, and EEXIST is
 * reported before EACCES.
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
    int key = own_key(1);
    i64 id = msgget(key, 0640 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    i64 denied = root ? id : -EACCES;
    i64 r = msgget(key, 0);
    check("ask_nothing_ok", r == id, r);
    r = msgget(key, 0600);
    check("owner_rw_ok", r == id, r);
    r = msgget(key, 0066);
    check("group_other_bits_check_owner", r == id, r);
    r = msgget(key, 0100);
    check("owner_x_denied", r == denied, r);
    r = msgget(key, 0010);
    check("group_x_bit_denied", r == denied, r);
    r = msgget(key, 0700 | IPC_CREAT);
    check("creat_existing_checks_access", r == denied, r);
    r = msgget(key, 0700 | IPC_CREAT | IPC_EXCL);
    check("eexist_before_eacces", r == -EEXIST, r);
    msg_rm(id);

    key = own_key(2);
    id = msgget(key, 0200 | IPC_CREAT | IPC_EXCL);
    check("wo_create_ok", id >= 0, id);
    denied = root ? id : -EACCES;
    r = msgget(key, 0222);
    check("wo_write_ok", r == id, r);
    r = msgget(key, 0400);
    check("wo_read_denied", r == denied, r);
    r = msgget(key, 0004);
    check("wo_other_r_bit_denied", r == denied, r);
    msg_rm(id);

    key = own_key(3);
    id = msgget(key, IPC_CREAT | IPC_EXCL);
    check("mode0_create_ok", id >= 0, id);
    denied = root ? id : -EACCES;
    check("mode0_ask_nothing_ok", msgget(key, 0) == id, 0);
    r = msgget(key, 0200);
    check("mode0_write_denied", r == denied, r);
    check("mode0_rm_by_owner_ok", msg_rm(id) == 0, 0);
    return finish();
}
