/* IPC_SET with bad input: uid or gid (u32) -1 is not a valid id: EINVAL,
 * and nothing of the buffer is applied. The whole 120-byte buffer is read
 * before anything else: NULL, or a buffer whose last 8 bytes are
 * unmapped, is EFAULT, also for an id that does not exist (but a
 * negative id is EINVAL first). A buffer in read-only memory works:
 * IPC_SET only reads it.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
/* asm-generic/errno.h */
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
#define IPC_64 0x0100
#define MSG_STAT 11
#define MSG_INFO 12
#define MSG_STAT_ANY 13

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

/* struct msginfo (linux/msg.h): 7 ints and a short, 32 bytes. */
struct k_msginfo {
    int msgpool, msgmap, msgmax, msgmnb, msgmni, msgssz, msgtql;
    unsigned short msgseg;
    unsigned short pad;
};

/* struct msgbuf (linux/msg.h) with a small text. */
struct k_msg {
    i64 mtype;
    char mtext[64];
};

static __attribute__((unused)) i64 msgget(i64 key, i64 flg) { return syscall2(SYS_msgget, key, flg); }
static __attribute__((unused)) i64 msgctl(i64 id, i64 cmd, const void *buf) { return syscall3(SYS_msgctl, id, cmd, buf); }
static __attribute__((unused)) i64 msq_rm(i64 id) { return msgctl(id, IPC_RMID, 0); }
static __attribute__((unused)) i64 msq_stat(i64 id, struct k_msqid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return msgctl(id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 msq_send(i64 id, i64 type, u64 len, i64 flg) {
    struct k_msg m;
    memset(&m, 'm', sizeof m);
    m.mtype = type;
    return syscall4(SYS_msgsnd, id, &m, len, flg);
}
static __attribute__((unused)) i64 msq_recv(i64 id, i64 flg) {
    struct k_msg m;
    memset(&m, 0, sizeof m);
    return syscall5(SYS_msgrcv, id, &m, sizeof m.mtext, 0, flg);
}
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x50000000 | (tag << 22) | (int)getpid()); }

int test_main(void) {
    i64 euid = syscall0(SYS_geteuid);
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("create_ok", id >= 0, id);

    struct k_msqid_ds ds;
    msq_stat(id, &ds);
    ds.uid = 0xffffffffU;
    ds.mode = 0644;
    i64 r = msgctl(id, IPC_SET, &ds);
    check("uid_minus1_einval", r == -EINVAL, r);
    msq_stat(id, &ds);
    check("uid_kept", ds.uid == euid, ds.uid);
    check("mode_kept_after_uid", ds.mode == 0600, ds.mode);

    ds.gid = 0xffffffffU;
    ds.mode = 0644;
    ds.qbytes = 10;
    r = msgctl(id, IPC_SET, &ds);
    check("gid_minus1_einval", r == -EINVAL, r);
    msq_stat(id, &ds);
    check("gid_kept", ds.gid == syscall0(SYS_getegid), ds.gid);
    check("mode_kept_after_gid", ds.mode == 0600, ds.mode);
    check("qbytes_kept_after_gid", ds.qbytes == 16384, (i64)ds.qbytes);

    r = msgctl(id, IPC_SET, 0);
    check("null_efault", r == -EFAULT, r);
    r = msgctl(id, IPC_SET, (void *)8);
    check("low_address_efault", r == -EFAULT, r);
    r = msgctl(0x7ffffff0, IPC_SET, 0);
    check("null_with_unknown_id_efault", r == -EFAULT, r);
    r = msgctl(-1, IPC_SET, 0);
    check("null_with_negative_id_einval", r == -EINVAL, r);

    /* Two pages, the second unmapped: the struct's last 8 bytes there. */
    i64 m = syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", m > 0, m);
    char *page = (char *)m;
    syscall2(SYS_munmap, page + 4096, 4096);
    struct k_msqid_ds *edge = (struct k_msqid_ds *)(page + 4096 - sizeof(struct k_msqid_ds) + 8);
    msq_stat(id, &ds);
    ds.mode = 0644;
    ds.qbytes = 1000;
    memcpy(edge, &ds, sizeof ds - 8);
    r = msgctl(id, IPC_SET, edge);
    check("tail_unmapped_efault", r == -EFAULT, r);
    msq_stat(id, &ds);
    check("tail_mode_kept", ds.mode == 0600, ds.mode);
    check("tail_qbytes_kept", ds.qbytes == 16384, (i64)ds.qbytes);

    /* The whole struct just fits: fine. */
    struct k_msqid_ds *fit = (struct k_msqid_ds *)(page + 4096 - sizeof(struct k_msqid_ds));
    msq_stat(id, &ds);
    ds.mode = 0640;
    memcpy(fit, &ds, sizeof ds);
    r = msgctl(id, IPC_SET, fit);
    check("fits_ok", r == 0, r);

    /* Read-only memory is enough for IPC_SET. */
    msq_stat(id, &ds);
    ds.mode = 0604;
    ds.qbytes = 2000;
    memcpy(page, &ds, sizeof ds);
    syscall3(SYS_mprotect, page, 4096, PROT_READ);
    r = msgctl(id, IPC_SET, page);
    check("read_only_buffer_ok", r == 0, r);
    msq_stat(id, &ds);
    check("read_only_mode", ds.mode == 0604, ds.mode);
    check("read_only_qbytes", ds.qbytes == 2000, (i64)ds.qbytes);

    check("rm_ok", msq_rm(id) == 0, 0);
    return finish();
}
