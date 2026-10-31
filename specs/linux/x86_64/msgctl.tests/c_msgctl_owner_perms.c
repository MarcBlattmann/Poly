/* msgctl for a caller that is not the owner (a child dropped to uid/gid
 * 65534, no capabilities): IPC_STAT and MSG_STAT need r in the queue's
 * "other" bits (EACCES), MSG_STAT_ANY needs nothing, IPC_SET and
 * IPC_RMID need euid == uid or cuid (EPERM, whatever the mode). Root
 * (CAP_SYS_RESOURCE) may raise msg_qbytes above 16384; the queue then
 * holds more than 16384 bytes.
 * poly-diff: needs=root (setresuid to another user; raising msg_qbytes)
 * poly-diff: syscalls=off (several processes)
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
    i64 a = msgget(IPC_PRIVATE, 0600);
    i64 b = msgget(IPC_PRIVATE, 0666);
    check("msgget_ok", a >= 0 && b >= 0, a);
    i64 pid = fork();
    if (pid == 0) {
        if (syscall3(SYS_setresgid, 65534, 65534, 65534) != 0) exit_group(100);
        if (syscall3(SYS_setresuid, 65534, 65534, 65534) != 0) exit_group(101);
        struct k_msqid_ds ds;
        int bad = 0;
        if (msg_stat(a, &ds) != -EACCES) bad |= 1;
        if (msgctl(a & 0x7fff, MSG_STAT, &ds) != -EACCES) bad |= 2;
        if (msgctl(a & 0x7fff, MSG_STAT_ANY, &ds) != a) bad |= 4;
        if (msg_rm(a) != -EPERM) bad |= 8;
        if (msg_stat(b, &ds) != 0) bad |= 16;
        if (msgctl(b, IPC_SET, &ds) != -EPERM) bad |= 32;
        if (msg_rm(b) != -EPERM) bad |= 64;
        if (send_text(b, 1, "hi", 2, IPC_NOWAIT) != 0) bad |= 128;
        exit_group(bad);
    }
    i64 st = wait_exit(pid);
    check("other_user_checks", st == 0, st);
    struct k_msqid_ds ds;
    msg_stat(b, &ds);
    check("other_user_message", ds.qnum == 1 && ds.lspid == pid, (i64)ds.qnum);

    msg_stat(a, &ds);
    ds.qbytes = 40000;
    i64 r = msgctl(a, IPC_SET, &ds);
    check("root_raises_qbytes", r == 0, r);
    int sent = 0;
    while (sent < 10 && send_text(a, 1, 0, 8192, IPC_NOWAIT) == 0) sent++;
    check("four_full_messages", sent == 4, sent);
    msg_stat(a, &ds);
    check("cbytes_32768", ds.cbytes == 32768 && ds.qbytes == 40000, (i64)ds.cbytes);
    msg_rm(a);
    msg_rm(b);
    return finish();
}
