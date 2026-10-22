/* personality: with READ_IMPLIES_EXEC (0x0400000) memory that mmap or
 * mprotect makes readable (PROT_READ without PROT_EXEC) can be executed;
 * without it, executing such memory kills the process with SIGSEGV.
 * Each case runs in a child that calls a 6-byte function returning 42.
 * poly-diff: syscalls=off (child processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_personality 135
/* linux/personality.h */
#define READ_IMPLIES_EXEC 0x0400000

#define F "/tmp/a_personality_rie"

static i64 personality(u64 p) { return syscall1(SYS_personality, p); }

/* mov $42, %eax; ret */
static const unsigned char code[] = {0xb8, 0x2a, 0x00, 0x00, 0x00, 0xc3};

enum { RW_THEN_READ, RW, FILE_READ };

/* In a child with persona `p`, get the code into a page in the given way
 * and call it. Returns the child's exit status (42 when the call worked)
 * or 1000 + the signal that killed it. */
static i64 run(u64 p, int how) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        personality(p);
        unsigned char *pg;
        if (how == FILE_READ) {
            i64 fd = a_open(F, O_RDONLY, 0);
            pg = (unsigned char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE, fd, 0);
        } else {
            pg = (unsigned char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS,
                                           -1, 0);
            memcpy(pg, code, sizeof code);
            if (how == RW_THEN_READ) syscall3(SYS_mprotect, pg, A_PAGE, PROT_READ);
        }
        int (*fn)(void) = (int (*)(void))pg;
        syscall1(SYS_exit_group, fn());
    }
    i32 st = -1;
    if (syscall4(SYS_wait4, pid, &st, 0, 0) != pid) return -1;
    if (A_WIFSIGNALED(st)) return 1000 + A_WTERMSIG(st);
    return A_WEXITSTATUS(st);
}

int test_main(void) {
    a_block_sigchld();
    i64 fd = a_open(F, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    check("setup_file", fd >= 0 && a_write(fd, code, sizeof code) == sizeof code, fd);
    a_close(fd);
    expect("plain_mprotect_read_faults", run(0, RW_THEN_READ), 1000 + SIGSEGV);
    expect("plain_rw_faults", run(0, RW), 1000 + SIGSEGV);
    expect("plain_file_read_faults", run(0, FILE_READ), 1000 + SIGSEGV);
    expect("rie_mprotect_read_executes", run(READ_IMPLIES_EXEC, RW_THEN_READ), 42);
    expect("rie_rw_executes", run(READ_IMPLIES_EXEC, RW), 42);
    expect("rie_file_read_executes", run(READ_IMPLIES_EXEC, FILE_READ), 42);
    expect("rie_with_domain_executes", run(READ_IMPLIES_EXEC | 0x0008, RW_THEN_READ), 42);
    expect("other_flags_fault", run(0xfffffffeUL & ~(u64)READ_IMPLIES_EXEC, RW_THEN_READ), 1000 + SIGSEGV);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
