/* Order of the checks when several things are wrong at once: first the
 * mtype is read (EFAULT for an unreadable msgp, before anything else);
 * then msgsz > msgmax or a negative msqid (EINVAL); then mtype < 1
 * (EINVAL); then the text is read (EFAULT); then the queue is looked up
 * (EINVAL for an unknown or removed id); then write permission (EACCES);
 * then room in the queue (EAGAIN with IPC_NOWAIT).
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
#define BAD_ID 0x7ffffff0

static struct k_msg good, zero_type;

int test_main(void) {
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    i64 gone = msgget(IPC_PRIVATE, 0600);
    msq_rm(gone);
    good.mtype = 1;
    zero_type.mtype = 0;

    /* A message whose text runs into an inaccessible page after 10 bytes. */
    i64 base = syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall3(SYS_mprotect, base + 4096, 4096, PROT_NONE);
    i64 *badtext = (i64 *)(base + 4096 - 18);
    *badtext = 1;

    /* mtype unreadable beats everything. */
    i64 r = msgsnd(-1, 0, 9000, IPC_NOWAIT);
    check("efault_before_size_and_negid", r == -EFAULT, r);
    r = msgsnd(BAD_ID, 0, 1, IPC_NOWAIT);
    check("efault_before_bad_id", r == -EFAULT, r);
    r = msgsnd(gone, 0, 1, IPC_NOWAIT);
    check("efault_before_removed_id", r == -EFAULT, r);

    /* Size and negative id beat the mtype and the text. */
    r = msgsnd(id, &zero_type, 9000, IPC_NOWAIT);
    check("size_with_type_0", r == -EINVAL, r);
    r = msgsnd(id, badtext, 9000, IPC_NOWAIT);
    check("size_before_text_fault", r == -EINVAL, r);
    r = msgsnd(-1, badtext, 11, IPC_NOWAIT);
    check("negid_before_text_fault", r == -EINVAL, r);

    /* mtype < 1 beats the text fault. */
    *badtext = 0;
    r = msgsnd(id, badtext, 11, IPC_NOWAIT);
    check("type_before_text_fault", r == -EINVAL, r);
    *badtext = 1;

    /* The text fault beats the id lookup. */
    r = msgsnd(BAD_ID, badtext, 11, IPC_NOWAIT);
    check("text_fault_before_bad_id", r == -EFAULT, r);
    r = msgsnd(gone, badtext, 11, IPC_NOWAIT);
    check("text_fault_before_removed_id", r == -EFAULT, r);

    /* The id lookup comes after the type check. */
    r = msgsnd(BAD_ID, &zero_type, 1, IPC_NOWAIT);
    check("type_0_bad_id", r == -EINVAL, r);

    /* Permission: after the text fault and the type, before a full queue. */
    i64 ro = msgget(IPC_PRIVATE, 0400);
    check("ro_msgget_ok", ro >= 0, ro);
    r = msgsnd(ro, badtext, 11, IPC_NOWAIT);
    check("text_fault_before_eacces", r == -EFAULT, r);
    r = msgsnd(ro, &zero_type, 1, IPC_NOWAIT);
    check("type_before_eacces", r == -EINVAL, r);
    r = msgsnd(ro, &good, 9000, IPC_NOWAIT);
    check("size_before_eacces", r == -EINVAL, r);
    msq_set(id, 4, 0400);
    r = msgsnd(id, &good, 8, IPC_NOWAIT);
    check("eacces_before_eagain", r == -EACCES, r);
    msq_set(id, 4, 0600);
    r = msgsnd(id, &good, 8, IPC_NOWAIT);
    check("eagain_last", r == -EAGAIN, r);
    check("nothing_queued", msq_qnum(id) == 0, msq_qnum(id));

    msq_rm(id);
    msq_rm(ro);
    return finish();
}
