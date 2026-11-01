/* msgsz is the room for text in the caller's buffer. A message longer
 * than msgsz is E2BIG and stays in the queue, unless MSG_NOERROR: then
 * the first msgsz bytes are copied, the call returns msgsz and the
 * rest of the message is gone. The return value is the number of text
 * bytes copied; bytes after them are not touched. msgsz may be larger
 * than msgmax; a msgsz with the top bit set is EINVAL. mtype is always
 * written.
 * poly-diff: syscalls=names (queue ids depend on the IPC namespace's history)
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
    static struct k_msg m;
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    send_text(id, 2, "0123456789", 10, 0);
    i64 r = msgrcv(id, &m, 9, 0, IPC_NOWAIT);
    check("short_buffer_e2big", r == -E2BIG, r);
    struct k_msqid_ds ds;
    msg_stat(id, &ds);
    check("message_stays", ds.qnum == 1 && ds.cbytes == 10, (i64)ds.qnum);
    check("e2big_lrpid_0", ds.lrpid == 0 && ds.rtime == 0, ds.lrpid);
    memset(m.mtext, '#', 16);
    r = msgrcv(id, &m, 4, 0, IPC_NOWAIT | MSG_NOERROR);
    check("noerror_truncates", r == 4, r);
    check("noerror_text", mem_eq(m.mtext, "0123", 4), 0);
    check("noerror_rest_untouched", m.mtext[4] == '#', m.mtext[4]);
    check("noerror_type", m.mtype == 2, m.mtype);
    msg_stat(id, &ds);
    check("truncated_message_gone", ds.qnum == 0 && ds.cbytes == 0, (i64)ds.qnum);

    send_text(id, 3, "abc", 3, 0);
    memset(m.mtext, '#', 16);
    r = msgrcv(id, &m, 100000, 0, IPC_NOWAIT);
    check("big_buffer_ok", r == 3 && mem_eq(m.mtext, "abc#", 4), r);

    send_text(id, 4, 0, 0, 0);
    r = msgrcv(id, &m, 0, 0, IPC_NOWAIT);
    check("zero_length_into_zero", r == 0 && m.mtype == 4, r);
    send_text(id, 4, "z", 1, 0);
    r = msgrcv(id, &m, 0, 0, IPC_NOWAIT);
    check("one_byte_into_zero_e2big", r == -E2BIG, r);
    r = msgrcv(id, &m, 0, 0, IPC_NOWAIT | MSG_NOERROR);
    check("one_byte_into_zero_noerror", r == 0 && m.mtype == 4, r);
    msg_stat(id, &ds);
    check("empty_again", ds.qnum == 0, (i64)ds.qnum);

    send_text(id, 5, "q", 1, 0);
    r = msgrcv(id, &m, 0x8000000000000000UL, 0, IPC_NOWAIT);
    check("negative_size_einval", r == -EINVAL, r);
    r = msgrcv(id, &m, 0xffffffffffffffffUL, 0, IPC_NOWAIT);
    check("minus_one_size_einval", r == -EINVAL, r);
    r = msgrcv(id, &m, (1UL << 32) | 1, 0, IPC_NOWAIT);
    check("size_64_bit_ok", r == 1, r);
    msg_rm(id);
    return finish();
}
