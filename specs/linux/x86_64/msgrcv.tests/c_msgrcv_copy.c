/* MSG_COPY (040000): msgtyp is a 0-based position in the queue and the
 * message there is copied out without being removed (qnum, cbytes,
 * lrpid, rtime unchanged). It needs IPC_NOWAIT (EINVAL without it) and
 * cannot be combined with MSG_EXCEPT (EINVAL). A position at or past the
 * end, or a negative one, is ENOMSG. A message longer than msgsz is
 * E2BIG; with MSG_NOERROR it is EINVAL (not truncated). The buffer must
 * be accessible for min(msgsz, msgmax) bytes even if the message is
 * shorter (EFAULT otherwise, the message stays).
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
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);

    i64 r = msgrcv(id, &out, 100, 0, IPC_NOWAIT | MSG_COPY);
    check("empty_enomsg", r == -ENOMSG, r);

    send_fill(id, 7, 'a', 1, 0);
    send_fill(id, 3, 'b', 2, 0);
    send_fill(id, 5, 'c', 3, 0);

    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT | MSG_COPY);
    check("pos0", r == 1 && out.mtype == 7 && out.mtext[0] == 'a', r);
    r = msgrcv(id, &out, 100, 2, IPC_NOWAIT | MSG_COPY);
    check("pos2", r == 3 && out.mtype == 5 && mem_eq(out.mtext, "ccc", 3), r);
    r = msgrcv(id, &out, 100, 1, IPC_NOWAIT | MSG_COPY);
    check("pos1", r == 2 && out.mtype == 3, r);
    r = msgrcv(id, &out, 100, 1, IPC_NOWAIT | MSG_COPY);
    check("pos1_again", r == 2 && out.mtype == 3, r);
    r = msgrcv(id, &out, 100, 3, IPC_NOWAIT | MSG_COPY);
    check("pos3_enomsg", r == -ENOMSG, r);
    r = msgrcv(id, &out, 100, -1, IPC_NOWAIT | MSG_COPY);
    check("negative_enomsg", r == -ENOMSG, r);
    r = msgrcv(id, &out, 100, (1L << 32) + 1, IPC_NOWAIT | MSG_COPY);
    check("pos_2pow32_plus_1_enomsg", r == -ENOMSG, r);

    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    check("not_removed", ds.qnum == 3 && ds.cbytes == 6, (i64)ds.qnum);
    check("lrpid_unchanged", ds.lrpid == 0 && ds.rtime == 0, ds.lrpid);

    r = msgrcv(id, &out, 100, 0, MSG_COPY);
    check("needs_nowait", r == -EINVAL, r);
    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT | MSG_COPY | MSG_EXCEPT);
    check("not_with_except", r == -EINVAL, r);
    r = msgrcv(id, &out, 100, 0, MSG_COPY | MSG_EXCEPT);
    check("neither", r == -EINVAL, r);

    r = msgrcv(id, &out, 2, 2, IPC_NOWAIT | MSG_COPY);
    check("too_small_e2big", r == -E2BIG, r);
    r = msgrcv(id, &out, 0, 0, IPC_NOWAIT | MSG_COPY);
    check("size0_e2big", r == -E2BIG, r);
    r = msgrcv(id, &out, 2, 2, IPC_NOWAIT | MSG_COPY | MSG_NOERROR);
    check("noerror_einval", r == -EINVAL, r);
    r = msgrcv(id, &out, 3, 2, IPC_NOWAIT | MSG_COPY | MSG_NOERROR);
    check("noerror_fits_ok", r == 3, r);
    r = msgrcv(id, &out, 1L << 40, 2, IPC_NOWAIT | MSG_COPY);
    check("huge_size_ok", r == 3, r);

    i64 page = syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall3(SYS_mprotect, page + 4096, 4096, PROT_NONE);
    r = msgrcv(id, (void *)(page + 4096 - 16), 100, 0, IPC_NOWAIT | MSG_COPY);
    check("short_room_efault", r == -EFAULT, r);
    r = msgrcv(id, (void *)(page + 4096 - 100), 100, 0, IPC_NOWAIT | MSG_COPY);
    check("room_msgsz_ok", r == 1, r);
    r = msgrcv(id, (void *)(page + 4096 - 16), 9, 0, IPC_NOWAIT | MSG_COPY);
    check("room_9_ok", r == 1, r);
    r = msgrcv(id, (void *)(page + 4096 - 8), 1, 0, IPC_NOWAIT | MSG_COPY);
    check("room_8_efault", r == -EFAULT, r);
    r = msgrcv(id, 0, 100, 0, IPC_NOWAIT | MSG_COPY);
    check("null_efault", r == -EFAULT, r);
    check("still_3", msq_qnum(id) == 3, msq_qnum(id));

    /* A normal receive afterwards still sees the queue in order. */
    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT);
    check("normal_first", r == 1 && out.mtype == 7, r);
    r = msgrcv(id, &out, 100, 0, IPC_NOWAIT | MSG_COPY);
    check("pos0_now_second", r == 2 && out.mtype == 3, r);

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
