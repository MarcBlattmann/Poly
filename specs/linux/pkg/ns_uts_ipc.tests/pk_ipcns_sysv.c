/* System V IPC objects live in an IPC namespace. A child made with
 * clone(CLONE_NEWIPC) starts with no message queues, shared memory
 * segments or semaphore sets: its first object of each kind has id 0,
 * /proc/sysvipc/{msg,shm,sem} list only the header line and that object,
 * and a lookup (msgget, shmget, semget with flag 0) of a key that exists
 * in the namespace it came from is ENOENT; creating an object with that
 * same key makes another one. Objects made before the namespace was
 * created are untouched by what happens inside (a queue removed there
 * with the same key leaves the outer one), objects of the outer namespace
 * cannot be reached by id either (msgctl IPC_STAT with the outer id is
 * EINVAL), and messages sent to a queue are only received in the
 * namespace that has it. After the inner namespace is left the outer
 * queue still holds its message. Two processes sharing the namespace
 * (fork) see each other's objects.
 * poly-diff: needs=root (a new IPC namespace needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes; ids depend on the system)
 */
#include "p_ns.h"

#define SYS_shmget 29
#define SYS_shmctl 31
#define SYS_semget 64
#define SYS_semctl 66
#define SYS_msgget 68
#define SYS_msgsnd 69
#define SYS_msgrcv 70
#define SYS_msgctl 71
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define ENOMSG 42
#define IPC_RMID 0
#define IPC_STAT 2
#define IPC_NOWAIT 04000

static i64 key;
static i64 outer_msg, outer_msg2, outer_shm, outer_sem;
static char buf[4096];

static int lines(const char *path) {
    i64 n = p_read_file(path, buf, sizeof buf);
    if (n < 0) return (int)n;
    int l = 0;
    for (i64 i = 0; i < n; i++)
        if (buf[i] == '\n') l++;
    return l;
}

static int inner(void) {
    expect("inner_msg_lookup_enoent", syscall2(SYS_msgget, key, 0), -ENOENT);
    expect("inner_shm_lookup_enoent", syscall3(SYS_shmget, key, 4096, 0), -ENOENT);
    expect("inner_sem_lookup_enoent", syscall3(SYS_semget, key, 1, 0), -ENOENT);
    expect("inner_msg_list_empty", lines("/proc/sysvipc/msg"), 1);
    expect("inner_shm_list_empty", lines("/proc/sysvipc/shm"), 1);
    expect("inner_sem_list_empty", lines("/proc/sysvipc/sem"), 1);

    expect("inner_msg_create_id_0", syscall2(SYS_msgget, key, IPC_CREAT | IPC_EXCL | 0600), 0);
    expect("inner_shm_create_id_0", syscall3(SYS_shmget, key, 4096, IPC_CREAT | IPC_EXCL | 0600), 0);
    expect("inner_sem_create_id_0", syscall3(SYS_semget, key, 1, IPC_CREAT | IPC_EXCL | 0600), 0);
    expect("inner_msg_list_one", lines("/proc/sysvipc/msg"), 2);
    expect("inner_shm_list_one", lines("/proc/sysvipc/shm"), 2);
    expect("inner_sem_list_one", lines("/proc/sysvipc/sem"), 2);

    char msgbuf[256];
    expect("inner_outer_second_queue_unknown", syscall3(SYS_msgctl, outer_msg2, IPC_STAT, msgbuf), -EINVAL);
    i64 mtype = 1;
    (void)mtype;
    char rb[24];
    expect("inner_queue_empty", syscall5(SYS_msgrcv, 0, rb, 8, 0, IPC_NOWAIT), -ENOMSG);

    expect("inner_msg_rmid", syscall3(SYS_msgctl, 0, IPC_RMID, 0), 0);
    expect("inner_shm_rmid", syscall3(SYS_shmctl, 0, IPC_RMID, 0), 0);
    expect("inner_sem_rmid", syscall4(SYS_semctl, 0, 0, IPC_RMID, 0), 0);
    expect("inner_msg_list_empty_again", lines("/proc/sysvipc/msg"), 1);
    return failed ? 1 : 0;
}

static int forked(void) {
    expect("fork_sees_msg", syscall2(SYS_msgget, key, 0), outer_msg);
    expect("fork_sees_shm", syscall3(SYS_shmget, key, 4096, 0), outer_shm);
    expect("fork_sees_sem", syscall3(SYS_semget, key, 1, 0), outer_sem);
    return failed ? 1 : 0;
}

static int body(void) {
    outer_msg = syscall2(SYS_msgget, key, IPC_CREAT | IPC_EXCL | 0600);
    outer_shm = syscall3(SYS_shmget, key, 4096, IPC_CREAT | IPC_EXCL | 0600);
    outer_sem = syscall3(SYS_semget, key, 1, IPC_CREAT | IPC_EXCL | 0600);
    outer_msg2 = syscall2(SYS_msgget, key + 1, IPC_CREAT | IPC_EXCL | 0600);
    expect("outer_msg_id_0", outer_msg, 0);
    expect("outer_msg2_id_1", outer_msg2, 1);
    expect("outer_shm_id_0", outer_shm, 0);
    expect("outer_sem_id_0", outer_sem, 0);
    struct {
        i64 mtype;
        char text[8];
    } m = {1, "hello"};
    expect("send", syscall4(SYS_msgsnd, outer_msg, &m, 8, 0), 0);
    expect("outer_msg_list_two", lines("/proc/sysvipc/msg"), 3);

    p_run("forked", 0, forked);
    p_run("inner_ns", CLONE_NEWIPC, inner);

    expect("outer_msg_still", syscall2(SYS_msgget, key, 0), outer_msg);
    expect("outer_msg_list_still_two", lines("/proc/sysvipc/msg"), 3);
    struct {
        i64 mtype;
        char text[8];
    } r = {0, ""};
    expect("recv_len", syscall5(SYS_msgrcv, outer_msg, &r, 8, 0, IPC_NOWAIT), 8);
    check("recv_text", str_eq(r.text, "hello"), 0);
    expect("outer_msg_rmid", syscall3(SYS_msgctl, outer_msg, IPC_RMID, 0), 0);
    expect("outer_msg2_rmid", syscall3(SYS_msgctl, outer_msg2, IPC_RMID, 0), 0);
    expect("outer_shm_rmid", syscall3(SYS_shmctl, outer_shm, IPC_RMID, 0), 0);
    expect("outer_sem_rmid", syscall4(SYS_semctl, outer_sem, 0, IPC_RMID, 0), 0);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    key = 0x706b0000 | (syscall0(SYS_getpid) & 0xffff);
    p_run("ipcns_sysv", CLONE_NEWIPC, body);
    return finish();
}
