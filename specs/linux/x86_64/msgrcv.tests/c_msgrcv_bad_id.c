/* The queue id: a negative id, an id never handed out and the id of a
 * removed queue are EINVAL (a call that does not sleep never sees
 * EIDRM), also with IPC_NOWAIT and with a NULL buffer (the id is checked
 * before the buffer is used). Only the low 32 bits of the id register
 * count. Flag bits msgrcv does not know are ignored.
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

int test_main(void) {
    i64 r = msgrcv(-1, &out, 100, 0, IPC_NOWAIT);
    check("negative_einval", r == -EINVAL, r);
    r = msgrcv(-1, &out, 100, 0, 0);
    check("negative_blocking_einval", r == -EINVAL, r);
    r = msgrcv(0x7ffffff0, &out, 100, 0, IPC_NOWAIT);
    check("unused_einval", r == -EINVAL, r);
    r = msgrcv(-1, 0, 100, 0, IPC_NOWAIT);
    check("null_buf_bad_id_einval", r == -EINVAL, r);

    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    send_fill(id, 1, 'a', 3, 0);
    r = msgrcv(id | (1L << 32), &out, 100, 0, IPC_NOWAIT);
    check("high_bits_ignored", r == 3, r);
    send_fill(id, 1, 'a', 3, 0);
    r = msgrcv(id | (0x1234L << 40), &out, 100, 0, IPC_NOWAIT);
    check("high_bits_ignored_2", r == 3, r);
    r = msgrcv(id | (1L << 32), &out, 100, 0, IPC_NOWAIT);
    check("high_bits_empty_enomsg", r == -ENOMSG, r);

    send_fill(id, 2, 'b', 3, 0);
    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT | 0x100000 | 0777 | 0200000);
    check("unknown_flags_ignored", r == 3 && out.mtype == 2, r);
    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT | 0x100000);
    check("unknown_flags_empty_enomsg", r == -ENOMSG, r);

    send_fill(id, 2, 'b', 3, 0);
    check("rm_ok", msq_rm(id) == 0, 0);
    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT);
    check("removed_einval", r == -EINVAL, r);
    r = msgrcv(id, &out, 100, 0, 0);
    check("removed_blocking_einval", r == -EINVAL, r);
    r = msgrcv(id, 0, 100, 0, IPC_NOWAIT);
    check("removed_null_einval", r == -EINVAL, r);
    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT | MSG_COPY);
    check("removed_copy_einval", r == -EINVAL, r);
    return finish();
}
