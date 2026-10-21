/* Message selection by msgtyp: 0 takes the first message of the queue;
 * > 0 the first message of exactly that type; < 0 the first message of
 * the lowest type that is <= |msgtyp| (send order among equal types).
 * msgtyp LONG_MIN selects like "lowest type of all". Nothing that
 * matches with IPC_NOWAIT is ENOMSG and leaves the queue as it was.
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

/* Receive with msgtyp `type`: the type got (text byte in *c), or -errno. */
static i64 take(i64 id, i64 type, char *c) {
    out.mtype = 0;
    out.mtext[0] = 0;
    i64 r = msgrcv(id, &out, 100, type, IPC_NOWAIT);
    *c = out.mtext[0];
    return r < 0 ? r : out.mtype;
}

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    char c = 0;

    /* queue: 3a 1b 2c 1d 5e 9f 4g */
    send_fill(id, 3, 'a', 1, 0);
    send_fill(id, 1, 'b', 1, 0);
    send_fill(id, 2, 'c', 1, 0);
    send_fill(id, 1, 'd', 1, 0);
    send_fill(id, 5, 'e', 1, 0);
    send_fill(id, 9, 'f', 1, 0);
    send_fill(id, 4, 'g', 1, 0);

    i64 t = take(id, 8, &c);
    check("type_8_none", t == -ENOMSG, t);
    check("none_keeps_queue", msq_qnum(id) == 7, msq_qnum(id));
    t = take(id, 1, &c);
    check("type_1_first", t == 1 && c == 'b', c);
    t = take(id, 1, &c);
    check("type_1_second", t == 1 && c == 'd', c);
    t = take(id, 1, &c);
    check("type_1_gone", t == -ENOMSG, t);

    /* queue: 3a 2c 5e 9f 4g */
    t = take(id, -4, &c);
    check("lowest_le_4_is_2", t == 2 && c == 'c', t);
    t = take(id, -1, &c);
    check("lowest_le_1_none", t == -ENOMSG, t);
    t = take(id, -3, &c);
    check("lowest_le_3_is_3", t == 3 && c == 'a', t);
    t = take(id, 0, &c);
    check("type_0_first", t == 5 && c == 'e', t);

    /* queue: 9f 4g */
    t = take(id, (i64)0x8000000000000000UL, &c);
    check("long_min_lowest", t == 4 && c == 'g', t);
    send_fill(id, 0x7fffffffffffffffL, 'h', 1, 0);
    t = take(id, 0x7fffffffffffffffL, &c);
    check("type_long_max", t == 0x7fffffffffffffffL && c == 'h', t);
    t = take(id, -0x7fffffffffffffffL, &c);
    check("lowest_le_long_max", t == 9 && c == 'f', t);

    /* Among equal lowest types the first sent wins. */
    send_fill(id, 6, 'i', 1, 0);
    send_fill(id, 2, 'j', 1, 0);
    send_fill(id, 7, 'k', 1, 0);
    send_fill(id, 2, 'l', 1, 0);
    t = take(id, -100, &c);
    check("lowest_first_sent", t == 2 && c == 'j', c);
    t = take(id, -100, &c);
    check("lowest_second_sent", t == 2 && c == 'l', c);
    t = take(id, -6, &c);
    check("then_6", t == 6 && c == 'i', t);
    t = take(id, -6, &c);
    check("7_too_high", t == -ENOMSG, t);
    check("one_left", msq_qnum(id) == 1, msq_qnum(id));

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
