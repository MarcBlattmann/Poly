/* PR_GET_TID_ADDRESS stores (as 8 bytes at arg2) the address last given
 * to set_tid_address or CLONE_CHILD_CLEARTID for this thread, 0 if none
 * (a started program has none); EFAULT if arg2 is not writable.
 * PR_GET_AUXV copies the process's auxiliary vector (the one on its
 * initial stack, 16-byte tag/value pairs ending in AT_NULL, followed by
 * zero bytes) into arg2, at most arg3 bytes, and returns the size of the
 * whole saved vector area (a multiple of 16, at least the vector's size)
 * whatever arg3 is; arg3 0 copies nothing (arg2 may then be NULL); a
 * buffer that cannot take the copy is EFAULT; non-zero arg4 or arg5 is
 * EINVAL.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_GET_TID_ADDRESS 40
#define PR_GET_AUXV 0x41555856

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static u64 av[128];

int test_main(void) {
    u64 t = 1;
    expect("tid address: get", pr(PR_GET_TID_ADDRESS, (i64)&t, 0, 0, 0), 0);
    expect("tid address: none at start", (i64)t, 0);
    i32 word = 0;
    syscall1(SYS_set_tid_address, &word);
    t = 1;
    pr(PR_GET_TID_ADDRESS, (i64)&t, 0, 0, 0);
    check("tid address: set_tid_address value", t == (u64)&word, 0);
    expect("tid address: NULL", pr(PR_GET_TID_ADDRESS, 0, 0, 0, 0), -EFAULT);
    expect("tid address: read-only", pr(PR_GET_TID_ADDRESS, (i64) "constant", 0, 0, 0), -EFAULT);
    syscall1(SYS_set_tid_address, 0);
    t = 1;
    pr(PR_GET_TID_ADDRESS, (i64)&t, 0, 0, 0);
    expect("tid address: cleared", (i64)t, 0);

    /* The vector on the initial stack. */
    u64 *sa = (u64 *)a_envp();
    while (*sa) sa++;
    sa++;
    i64 n = 0;
    while (sa[2 * n]) n++;
    i64 vsize = (n + 1) * 16;

    i64 total = pr(PR_GET_AUXV, 0, 0, 0, 0);
    check("auxv: size with NULL, 0", total >= vsize && total % 16 == 0 && total <= (i64)sizeof av, total);
    for (int i = 0; i < 128; i++) av[i] = 0x5a5a;
    expect("auxv: 16 bytes", pr(PR_GET_AUXV, (i64)av, 16, 0, 0), total);
    check("auxv: first pair", av[0] == sa[0] && av[1] == sa[1], (i64)av[0]);
    check("auxv: nothing past 16 bytes", av[2] == 0x5a5a, (i64)av[2]);
    for (int i = 0; i < 128; i++) av[i] = 0x5a5a;
    expect("auxv: big buffer", pr(PR_GET_AUXV, (i64)av, sizeof av, 0, 0), total);
    int same = 1;
    for (i64 i = 0; i < 2 * (n + 1); i++)
        if (av[i] != sa[i]) same = 0;
    expect("auxv: same as initial stack", same, 1);
    int zero = 1;
    for (i64 i = 2 * (n + 1); i < total / 8; i++)
        if (av[i]) zero = 0;
    expect("auxv: zeros after AT_NULL", zero, 1);
    check("auxv: nothing past the area", av[total / 8] == 0x5a5a, (i64)av[total / 8]);
    expect("auxv: NULL with size", pr(PR_GET_AUXV, 0, 64, 0, 0), -EFAULT);
    expect("auxv: arg4", pr(PR_GET_AUXV, (i64)av, sizeof av, 1, 0), -EINVAL);
    expect("auxv: arg5", pr(PR_GET_AUXV, (i64)av, sizeof av, 0, 1), -EINVAL);
    return finish();
}
