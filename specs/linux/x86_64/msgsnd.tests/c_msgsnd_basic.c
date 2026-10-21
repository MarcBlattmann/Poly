/* Basic sends: msgsnd returns 0 and appends the message; qnum counts
 * messages and cbytes the text bytes (the mtype is not counted). The
 * receiver gets back the same type and text, in send order. Sizes 0 and
 * 8192 (msgmax) work. The text is copied at the call: changing the
 * buffer afterwards does not change the queued message. A buffer in
 * read-only memory works.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
 */
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
static struct k_msg in, out;

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);

    in.mtype = 5;
    memcpy(in.mtext, "hello", 5);
    i64 r = msgsnd(id, &in, 5, 0);
    check("send_returns_0", r == 0, r);
    check("qnum_1", msq_qnum(id) == 1, msq_qnum(id));
    check("cbytes_5", msq_cbytes(id) == 5, msq_cbytes(id));

    memcpy(in.mtext, "XXXXX", 5);
    in.mtype = 6;
    r = msgsnd(id, &in, 0, 0);
    check("size_0_ok", r == 0, r);
    check("size_0_qnum", msq_qnum(id) == 2, msq_qnum(id));
    check("size_0_cbytes", msq_cbytes(id) == 5, msq_cbytes(id));

    for (int i = 0; i < 8192; i++) in.mtext[i] = (char)(i * 7 + 1);
    in.mtype = 7;
    r = msgsnd(id, &in, 8192, 0);
    check("size_max_ok", r == 0, r);
    check("size_max_cbytes", msq_cbytes(id) == 8197, msq_cbytes(id));
    memset(in.mtext, 0, 8192);

    memset(&out, 0, sizeof out);
    r = msgrcv(id, &out, sizeof out.mtext, 0, IPC_NOWAIT);
    check("first_len", r == 5, r);
    check("first_type", out.mtype == 5, out.mtype);
    check("first_text_copied", mem_eq(out.mtext, "hello", 5), 0);
    r = msgrcv(id, &out, sizeof out.mtext, 0, IPC_NOWAIT);
    check("second_len_0", r == 0, r);
    check("second_type", out.mtype == 6, out.mtype);
    r = msgrcv(id, &out, sizeof out.mtext, 0, IPC_NOWAIT);
    check("third_len", r == 8192, r);
    check("third_type", out.mtype == 7, out.mtype);
    int same = 1;
    for (int i = 0; i < 8192; i++)
        if (out.mtext[i] != (char)(i * 7 + 1)) same = 0;
    check("third_text", same, 0);
    check("empty_again", msq_qnum(id) == 0 && msq_cbytes(id) == 0, msq_qnum(id));

    /* A message in read-only memory. */
    i64 page = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    struct k_msg *ro = (struct k_msg *)page;
    ro->mtype = 9;
    memcpy(ro->mtext, "ro", 2);
    syscall3(SYS_mprotect, page, 4096, PROT_READ);
    r = msgsnd(id, ro, 2, IPC_NOWAIT);
    check("read_only_ok", r == 0, r);
    r = msgrcv(id, &out, sizeof out.mtext, 0, IPC_NOWAIT);
    check("read_only_back", r == 2 && out.mtype == 9 && mem_eq(out.mtext, "ro", 2), r);

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
