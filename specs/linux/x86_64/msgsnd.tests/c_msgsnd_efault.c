/* EFAULT: msgsnd reads the mtype (8 bytes) and msgsz bytes of text from
 * msgp. NULL, a low address and a non-canonical address are EFAULT, also
 * with msgsz 0 (the mtype is always read). A text that runs into an
 * unmapped page is EFAULT; one that ends right before it works (also an
 * mtype in the last 8 bytes with msgsz 0). An
 * mtype straddling into the unmapped page is EFAULT. Nothing is queued.
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
int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);

    i64 r = msgsnd(id, 0, 1, IPC_NOWAIT);
    check("null", r == -EFAULT, r);
    r = msgsnd(id, 0, 0, 0);
    check("null_size_0", r == -EFAULT, r);
    r = msgsnd(id, (void *)16, 1, IPC_NOWAIT);
    check("low_address", r == -EFAULT, r);
    r = msgsnd(id, (void *)0x800000000000UL, 1, IPC_NOWAIT);
    check("non_canonical", r == -EFAULT, r);
    r = msgsnd(id, (void *)0xffff800000000000UL, 1, IPC_NOWAIT);
    check("kernel_half", r == -EFAULT, r);

    /* Two pages, the second made inaccessible. */
    i64 base = syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", base > 0, base);
    syscall3(SYS_mprotect, base + 4096, 4096, PROT_NONE);
    char *end = (char *)base + 4096;
    i64 *msg = (i64 *)(end - 8 - 10);
    *msg = 3;
    memcpy((char *)(msg + 1), "0123456789", 10);
    r = msgsnd(id, msg, 10, IPC_NOWAIT);
    check("text_to_page_end_ok", r == 0, r);
    r = msgsnd(id, msg, 11, IPC_NOWAIT);
    check("text_past_page_end", r == -EFAULT, r);
    r = msgsnd(id, msg, 8000, IPC_NOWAIT);
    check("text_far_past_end", r == -EFAULT, r);
    r = msgsnd(id, end - 4, 0, IPC_NOWAIT);
    check("mtype_straddles", r == -EFAULT, r);
    *(i64 *)(end - 8) = 4;
    r = msgsnd(id, end - 8, 0, IPC_NOWAIT);
    check("mtype_at_page_end_size_0_ok", r == 0, r);
    check("two_queued", msq_qnum(id) == 2, msq_qnum(id));
    check("cbytes_10", msq_cbytes(id) == 10, msq_cbytes(id));

    msq_rm(id);
    return finish();
}
