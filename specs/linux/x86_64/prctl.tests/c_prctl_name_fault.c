/* PR_SET_NAME reads at most 15 bytes of the new name, stopping at a NUL:
 * a 15-byte name with no NUL that ends right before an unmapped page
 * works, a 14-byte one there fails with EFAULT (the 15th byte is read).
 * PR_GET_NAME writes 16 bytes: all of them must be writable, or EFAULT.
 * A NULL or read-only pointer is EFAULT. A failed call leaves the name.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_NAME 15
#define PR_GET_NAME 16

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static const char ro_buf[32] = "read-only data, not writable";

int test_main(void) {
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, m + A_PAGE, A_PAGE);
    char *end = m + A_PAGE;
    for (int i = 1; i <= 16; i++) end[-i] = (char)('a' + i - 1);
    char name[17];

    expect("set known name", pr(PR_SET_NAME, (i64) "known", 0, 0, 0), 0);
    expect("set NULL", pr(PR_SET_NAME, 0, 0, 0, 0), -EFAULT);
    expect("set unmapped", pr(PR_SET_NAME, (i64)end, 0, 0, 0), -EFAULT);
    expect("set 14 bytes before hole", pr(PR_SET_NAME, (i64)(end - 14), 0, 0, 0), -EFAULT);
    name[16] = 0;
    pr(PR_GET_NAME, (i64)name, 0, 0, 0);
    check("failed sets keep name", str_eq(name, "known"), name[0]);

    expect("set 15 bytes before hole", pr(PR_SET_NAME, (i64)(end - 15), 0, 0, 0), 0);
    pr(PR_GET_NAME, (i64)name, 0, 0, 0);
    check("15 bytes taken", str_eq(name, "onmlkjihgfedcba"), str_len(name));

    end[-1] = 0;
    expect("set 2 bytes + NUL before hole", pr(PR_SET_NAME, (i64)(end - 3), 0, 0, 0), 0);
    pr(PR_GET_NAME, (i64)name, 0, 0, 0);
    check("short name before hole", str_eq(name, "cb"), name[0]);

    expect("get NULL", pr(PR_GET_NAME, 0, 0, 0, 0), -EFAULT);
    expect("get 15 bytes before hole", pr(PR_GET_NAME, (i64)(end - 15), 0, 0, 0), -EFAULT);
    expect("get 3 bytes before hole", pr(PR_GET_NAME, (i64)(end - 3), 0, 0, 0), -EFAULT);
    expect("get read-only", pr(PR_GET_NAME, (i64)ro_buf, 0, 0, 0), -EFAULT);
    check("read-only untouched", ro_buf[0] == 'r', ro_buf[0]);
    expect("get 16 bytes before hole", pr(PR_GET_NAME, (i64)(end - 16), 0, 0, 0), 0);
    check("16 bytes written", str_eq(end - 16, "cb"), end[-16]);
    return finish();
}
