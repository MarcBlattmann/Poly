/* MSG_EXCEPT (020000): with msgtyp > 0 the first message whose type is
 * NOT msgtyp is taken; if every message has that type, ENOMSG. With
 * msgtyp 0 it changes nothing (first message); with msgtyp < 0 it is
 * ignored (lowest type <= |msgtyp|).
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EPERM 1
#define EINTR 4
#define E2BIG 7
#define EAGAIN 11
#define EACCES 13
#define ENOMSG 42
#define EIDRM 43
/* linux/ipc.h, linux/msg.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_NOWAIT 04000
#define MSG_NOERROR 010000
#define MSG_EXCEPT 020000
#define MSG_COPY 040000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2

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

/* struct msgbuf (linux/msg.h): a long type, then the text. */
struct k_msg {
    i64 mtype;
    char mtext[8200];
};

static __attribute__((unused)) i64 msgget(i64 key, i64 flg) { return syscall2(SYS_msgget, key, flg); }
static __attribute__((unused)) i64 msgsnd(i64 id, const void *msg, u64 size, i64 flg) {
    return syscall4(SYS_msgsnd, id, msg, size, flg);
}
static __attribute__((unused)) i64 msgrcv(i64 id, void *msg, u64 size, i64 type, i64 flg) {
    return syscall5(SYS_msgrcv, id, msg, size, type, flg);
}
static __attribute__((unused)) i64 msq_rm(i64 id) { return syscall3(SYS_msgctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 msq_stat(i64 id, struct k_msqid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_msgctl, id, IPC_STAT, ds);
}
/* IPC_SET with the queue's current settings but the given qbytes and mode. */
static __attribute__((unused)) i64 msq_set(i64 id, u64 qbytes, int mode) {
    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    ds.qbytes = qbytes;
    ds.mode = (unsigned short)mode;
    return syscall3(SYS_msgctl, id, IPC_SET, &ds);
}
static __attribute__((unused)) i64 msq_qnum(i64 id) {
    struct k_msqid_ds ds;
    return msq_stat(id, &ds) == 0 ? (i64)ds.qnum : -1;
}
static __attribute__((unused)) i64 msq_cbytes(i64 id) {
    struct k_msqid_ds ds;
    return msq_stat(id, &ds) == 0 ? (i64)ds.cbytes : -1;
}
/* Send `size` bytes of `fill` with type `type`. */
static __attribute__((unused)) i64 send_fill(i64 id, i64 type, char fill, u64 size, i64 flg) {
    static struct k_msg m;
    m.mtype = type;
    memset(m.mtext, fill, size < sizeof m.mtext ? size : sizeof m.mtext);
    return msgsnd(id, &m, size, flg);
}
/* Poll qnum until it is `want`; 0 after ~2 s. */
static __attribute__((unused)) int wait_qnum(i64 id, i64 want) {
    for (int i = 0; i < 2000; i++) {
        if (msq_qnum(id) == want) return 1;
        sleep_ms(1);
    }
    return 0;
}
static __attribute__((unused)) struct k_msg in, out;

static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
static volatile int caught;
static __attribute__((unused)) void on_signal(int sig) {
    (void)sig;
    caught++;
}
/* Fork a receiver: msgrcv(id, size, type, flg) with a handler for `sig`
 * (0: none) installed with `sflags`. It exits with 100 + errno on
 * failure, else with mtype * 10 + the returned length. */
static __attribute__((unused)) i64 receiver(i64 id, u64 size, i64 type, i64 flg, int sig, u64 sflags) {
    i64 pid = fork();
    if (pid == 0) {
        if (sig) sig_handler(sig, on_signal, sflags);
        static struct k_msg m;
        i64 r = msgrcv(id, &m, size, type, flg);
        exit_group(r < 0 ? (int)(100 - r) : (int)(m.mtype * 10 + r));
    }
    return pid;
}

static i64 take(i64 id, i64 type, i64 flg, char *c) {
    out.mtype = 0;
    out.mtext[0] = 0;
    i64 r = msgrcv(id, &out, 100, type, IPC_NOWAIT | flg);
    *c = out.mtext[0];
    return r < 0 ? r : out.mtype;
}

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    char c = 0;

    send_fill(id, 3, 'a', 1, 0);
    send_fill(id, 3, 'b', 1, 0);
    send_fill(id, 2, 'c', 1, 0);
    send_fill(id, 5, 'd', 1, 0);
    send_fill(id, 1, 'e', 1, 0);
    send_fill(id, 4, 'f', 1, 0);

    i64 t = take(id, 3, MSG_EXCEPT, &c);
    check("except_3_first_other", t == 2 && c == 'c', t);
    t = take(id, 3, MSG_EXCEPT, &c);
    check("except_3_next_other", t == 5 && c == 'd', t);
    t = take(id, 7, MSG_EXCEPT, &c);
    check("except_7_first", t == 3 && c == 'a', t);
    /* queue: 3b 1e 4f */
    t = take(id, 0, MSG_EXCEPT, &c);
    check("except_0_first", t == 3 && c == 'b', t);
    /* queue: 1e 4f */
    t = take(id, -5, MSG_EXCEPT, &c);
    check("except_negative_ignored", t == 1 && c == 'e', t);
    /* queue: 4f */
    t = take(id, 4, MSG_EXCEPT, &c);
    check("except_all_same_enomsg", t == -ENOMSG, t);
    check("still_one", msq_qnum(id) == 1, msq_qnum(id));
    send_fill(id, 4, 'g', 1, 0);
    t = take(id, 4, MSG_EXCEPT, &c);
    check("except_two_same_enomsg", t == -ENOMSG, t);
    t = take(id, 9, MSG_EXCEPT, &c);
    check("except_9_takes_4", t == 4 && c == 'f', t);

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
