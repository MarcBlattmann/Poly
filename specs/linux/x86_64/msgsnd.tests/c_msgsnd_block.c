/* Without IPC_NOWAIT a message that does not fit makes the sender sleep:
 * a receive that makes room lets it continue (its message is then
 * queued, lspid is the sender); removing the queue ends the sleep with
 * EIDRM; a signal with a handler ends it with EINTR (also with
 * SA_RESTART), and nothing is queued.
 * poly-diff: syscalls=off (several processes, timing-dependent)
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

static volatile int got;
static void on_usr1(int sig) {
    (void)sig;
    got++;
}

static void fill(i64 id) {
    send_text(id, 1, 0, 8192, IPC_NOWAIT);
    send_text(id, 1, 0, 8192, IPC_NOWAIT);
}

int test_main(void) {
    static struct k_msg m;
    struct k_msqid_ds ds;
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    fill(id);
    i64 pid = fork();
    if (pid == 0) exit_group(send_text(id, 9, "late", 4, 0) == 0 ? 0 : 1);
    sleep_ms(300);
    msg_stat(id, &ds);
    check("sender_sleeps", ds.qnum == 2, (i64)ds.qnum);
    i64 r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("make_room", r == 8192, r);
    check("sender_done", wait_exit(pid) == 0, 0);
    msg_stat(id, &ds);
    check("queued_after_room", ds.qnum == 2 && ds.cbytes == 8196, (i64)ds.cbytes);
    check("lspid_sender", ds.lspid == pid, 0);
    msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    r = msgrcv(id, &m, sizeof m.mtext, 0, IPC_NOWAIT);
    check("late_message_last", r == 4 && m.mtype == 9, r);
    msg_rm(id);

    id = msgget(IPC_PRIVATE, 0600);
    fill(id);
    pid = fork();
    if (pid == 0) exit_group(send_text(id, 1, 0, 10, 0) == -EIDRM ? 0 : 1);
    sleep_ms(300);
    check("rm_ok", msg_rm(id) == 0, 0);
    check("sender_eidrm", wait_exit(pid) == 0, 0);

    id = msgget(IPC_PRIVATE, 0600);
    fill(id);
    sig_handler(SIGUSR1, on_usr1, SA_RESTART);
    i64 parent = getpid();
    pid = fork();
    if (pid == 0) {
        sleep_ms(300);
        syscall2(SYS_kill, parent, SIGUSR1);
        exit_group(0);
    }
    r = send_text(id, 1, 0, 10, 0);
    check("sender_eintr", r == -EINTR, r);
    check("handler_ran", got == 1, got);
    msg_stat(id, &ds);
    check("nothing_queued", ds.qnum == 2, (i64)ds.qnum);
    wait_exit(pid);
    msg_rm(id);
    return finish();
}
