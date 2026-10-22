/* getresgid after set calls that keep the ids: setresgid(-1, -1, -1),
 * setresgid with the current ids, setregid(-1, -1), setgid and setfsgid
 * with the current gid all succeed (any user may do these) and getresgid
 * still reports the same three ids. */
#include "a_sys.h"

#define SYS_setgid 106    /* asm/unistd_64.h */
#define SYS_setregid 114  /* asm/unistd_64.h */
#define SYS_setresgid 119 /* asm/unistd_64.h */
#define SYS_getresgid 120 /* asm/unistd_64.h */
#define SYS_setfsgid 123  /* asm/unistd_64.h */

static u32 r0, e0, s0;

static int same(void) {
    u32 r = 1, e = 1, s = 1;
    return syscall3(SYS_getresgid, &r, &e, &s) == 0 && r == r0 && e == e0 && s == s0;
}

int test_main(void) {
    syscall3(SYS_getresgid, &r0, &e0, &s0);
    expect("setresgid_keep_all", syscall3(SYS_setresgid, -1, -1, -1), 0);
    check("after_keep_all", same(), 0);
    expect("setresgid_current", syscall3(SYS_setresgid, r0, e0, s0), 0);
    check("after_current", same(), 0);
    expect("setregid_keep", syscall2(SYS_setregid, -1, -1), 0);
    check("after_setregid_keep", same(), 0);
    expect("setgid_current", syscall1(SYS_setgid, e0), 0);
    check("after_setgid_current", same(), 0);
    /* setfsgid returns the previous fs gid, here the effective gid. */
    expect("setfsgid_current", syscall1(SYS_setfsgid, e0), e0);
    check("after_setfsgid", same(), 0);
    return finish();
}
