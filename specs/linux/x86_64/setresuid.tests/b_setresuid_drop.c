/* Dropping root for good with setresuid (in a child): once real,
 * effective and saved are all nonzero the permitted capabilities are gone
 * too and the process cannot get back to 0 (EPERM); an unprivileged
 * process may set each field to any of its current real, effective or
 * saved uids, in any order, and nothing else (EPERM, fields unchanged).
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

static u64 cap_perm(void) {
    struct {
        unsigned version;
        int pid;
    } h = { 0x20080522, 0 };
    unsigned d[6] = { 0 };
    syscall2(SYS_capget, &h, d);
    return d[1] | ((u64)d[4] << 32);
}

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        if (syscall3(SYS_setresuid, 1000, 2000, 3000) != 0) code |= 1;
        if (cap_perm() != 0) code |= 2;
        if (syscall3(SYS_setresuid, 0, -1, -1) != -EPERM) code |= 4;
        /* Permute among {1000, 2000, 3000}. */
        if (syscall3(SYS_setresuid, 3000, 1000, 2000) != 0) code |= 8;
        unsigned r, e, s;
        syscall3(SYS_getresuid, &r, &e, &s);
        if (r != 3000 || e != 1000 || s != 2000) code |= 16;
        if (syscall3(SYS_setresuid, -1, 4000, -1) != -EPERM) code |= 32;
        syscall3(SYS_getresuid, &r, &e, &s);
        if (e != 1000) code |= 64;
        if (syscall1(SYS_setuid, 0) != -EPERM) code |= 128;
        exit_group(code);
    }
    int code = wait_exit(pid);
    check("drop_all_ok", code >= 0 && !(code & 1), code);
    check("permitted_caps_gone", code >= 0 && !(code & 2), code);
    check("cannot_return_to_root", code >= 0 && !(code & 4), code);
    check("unprivileged_permutation_ok", code >= 0 && !(code & 8) && !(code & 16), code);
    check("unprivileged_new_uid_eperm", code >= 0 && !(code & 32) && !(code & 64), code);
    check("setuid_0_eperm", code >= 0 && !(code & 128), code);
    check("parent_still_root", syscall0(SYS_geteuid) == 0, 0);
    return finish();
}
