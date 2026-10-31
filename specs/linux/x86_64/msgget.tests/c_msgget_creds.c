/* A new queue's owner and creator are the caller's effective uid and gid,
 * not the real or saved ones: a child with real uid 1000, effective 5678,
 * saved 0 (and gids 2000/1234/0) makes a queue owned by 5678:1234.
 * Owner rights follow the effective uid: with mode 0600 it can stat
 * the queue; after it switches its effective uid back to 0 it is root.
 * poly-diff: needs=root (changing to arbitrary uids and gids)
 * poly-diff: syscalls=off (several processes)
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
    i64 pid = fork();
    if (pid == 0) {
        if (syscall3(SYS_setresgid, 2000, 1234, 0) != 0) exit_group(100);
        if (syscall3(SYS_setresuid, 1000, 5678, 0) != 0) exit_group(101);
        i64 id = msgget(IPC_PRIVATE, 0600);
        if (id < 0) exit_group(102);
        struct k_msqid_ds ds;
        int bad = 0;
        if (msg_stat(id, &ds) != 0) bad |= 1;
        if (ds.uid != 5678 || ds.cuid != 5678) bad |= 2;
        if (ds.gid != 1234 || ds.cgid != 1234) bad |= 4;
        if (send_text(id, 1, "x", 1, IPC_NOWAIT) != 0) bad |= 8;
        if (msg_rm(id) != 0) bad |= 16;
        exit_group(bad);
    }
    i64 st = wait_exit(pid);
    check("effective_ids_own_queue", st == 0, st);

    /* A queue made by root and given to 5678: owner rights for euid 5678. */
    i64 id = msgget(IPC_PRIVATE, 0600);
    check("msgget_ok", id >= 0, id);
    struct k_msqid_ds ds;
    msg_stat(id, &ds);
    ds.uid = 5678;
    ds.gid = 1234;
    check("give_away_ok", msgctl(id, IPC_SET, &ds) == 0, 0);
    pid = fork();
    if (pid == 0) {
        if (syscall3(SYS_setresgid, 1234, 1234, 1234) != 0) exit_group(100);
        if (syscall3(SYS_setresuid, 5678, 5678, 5678) != 0) exit_group(101);
        int bad = 0;
        if (send_text(id, 3, "abc", 3, IPC_NOWAIT) != 0) bad |= 1;
        if (msg_stat(id, &ds) != 0 || ds.qnum != 1) bad |= 2;
        exit_group(bad);
    }
    st = wait_exit(pid);
    check("new_owner_sends", st == 0, st);
    msg_stat(id, &ds);
    check("cuid_root_kept", ds.cuid == 0 && ds.uid == 5678, ds.uid);
    msg_rm(id);
    return finish();
}
