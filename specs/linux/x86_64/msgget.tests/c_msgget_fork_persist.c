/* Queues are system-wide, not per process: a child makes a keyed queue,
 * puts a message on it and exits; the queue outlives it, the parent
 * finds it by key and receives the child's message, and IPC_STAT names
 * the child as last sender. A queue made before fork is found by key in
 * the child, and what the child sends there reaches the parent.
 * poly-diff: syscalls=off (two processes)
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
    int key = own_key(15);
    int key2 = own_key(0);
    i64 pre = msgget(key2, 0600 | IPC_CREAT | IPC_EXCL);
    check("pre_create_ok", pre >= 0, pre);
    i64 pid = fork();
    if (pid == 0) {
        i64 id = msgget(key, 0600 | IPC_CREAT | IPC_EXCL);
        if (id < 0) exit_group(1);
        if (msq_send(id, 7, "child", 5) != 0) exit_group(2);
        if (msgget(key2, 0600) != pre) exit_group(3);
        if (msq_send(pre, 3, "hi", 2) != 0) exit_group(4);
        exit_group(0);
    }
    check("fork_ok", pid > 0, pid);
    int code = wait_exit(pid);
    check("child_ok", code == 0, code);

    i64 id = msgget(key, 0600);
    check("child_queue_found", id >= 0, id);
    struct k_msqid_ds ds;
    i64 r = msq_stat(id, &ds);
    check("stat_ok", r == 0, r);
    check("lspid_child", ds.lspid == pid, ds.lspid == 0 ? 0 : 1);
    check("qnum_one", ds.qnum == 1, (i64)ds.qnum);
    check("cuid_self", ds.cuid == syscall0(SYS_geteuid), ds.cuid);
    struct k_msg m;
    r = msq_recv(id, &m);
    check("recv_len", r == 5, r);
    check("recv_type", m.mtype == 7, m.mtype);
    check("recv_text", mem_eq(m.mtext, "child", 5), m.mtext[0]);

    r = msq_recv(pre, &m);
    check("pre_recv_len", r == 2, r);
    check("pre_recv_text", m.mtype == 3 && mem_eq(m.mtext, "hi", 2), m.mtype);
    msq_rm(id);
    msq_rm(pre);
    return finish();
}
