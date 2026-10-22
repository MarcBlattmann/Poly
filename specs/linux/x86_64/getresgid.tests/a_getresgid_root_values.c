/* getresgid reports what the set calls made: setresgid sets each id
 * (-1 keeps one), setgid by a privileged caller sets all three, setregid
 * also sets the saved id to the new effective id when the real id is
 * set, and setfsgid changes none of the three. Distinct ids show the
 * order real, effective, saved and the store order with one pointer.
 * poly-diff: needs=root (only root may set arbitrary group ids)
 */
#include "a_sys.h"

#define SYS_setgid 106    /* asm/unistd_64.h */
#define SYS_setregid 114  /* asm/unistd_64.h */
#define SYS_setresgid 119 /* asm/unistd_64.h */
#define SYS_getresgid 120 /* asm/unistd_64.h */
#define SYS_setfsgid 123  /* asm/unistd_64.h */

static u32 r, e, s;

static i64 ids(void) {
    r = e = s = 0xdeadbeef;
    syscall3(SYS_getresgid, &r, &e, &s);
    return (i64)r * 1000000 + (i64)e * 1000 + (i64)s;
}

int test_main(void) {
    expect("setresgid", syscall3(SYS_setresgid, 33, 44, 55), 0);
    expect("order_real_effective_saved", ids(), 33044055);
    expect("getgid_is_real", syscall0(SYS_getgid), 33);
    expect("getegid_is_effective", syscall0(SYS_getegid), 44);
    u32 one = 0;
    syscall3(SYS_getresgid, &one, &one, &one);
    expect("same_pointer_last_is_saved", one, 55);
    unsigned char b[6] = {0, 0, 0, 0, 0, 0};
    syscall3(SYS_getresgid, b, b + 1, b + 2);
    check("overlap_store_order", b[0] == 33 && b[1] == 44 && b[2] == 55 && b[3] == 0 && b[4] == 0 && b[5] == 0,
          b[0] * 10000 + b[1] * 100 + b[2]);
    expect("keep_real_and_saved", syscall3(SYS_setresgid, -1, 66, -1), 0);
    expect("after_keep", ids(), 33066055);
    expect("large_ids", syscall3(SYS_setresgid, 100000, 200000, 300000), 0);
    ids();
    check("large_ids_reported", r == 100000 && e == 200000 && s == 300000, (i64)r);
    /* setfsgid returns the previous fs gid, which setresgid made the effective one. */
    expect("setfsgid", syscall1(SYS_setfsgid, 77), 200000);
    ids();
    check("setfsgid_changes_none", r == 100000 && e == 200000 && s == 300000, (i64)e);
    expect("setgid_privileged", syscall1(SYS_setgid, 88), 0);
    expect("setgid_sets_all_three", ids(), 88088088);
    expect("setregid_real", syscall2(SYS_setregid, 10, 20), 0);
    expect("setregid_saved_follows_effective", ids(), 10020020);
    expect("setregid_effective_only_same_as_real", syscall2(SYS_setregid, -1, 10), 0);
    expect("effective_equal_real_keeps_saved", ids(), 10010020);
    expect("setregid_effective_only_other", syscall2(SYS_setregid, -1, 30), 0);
    expect("effective_not_real_sets_saved", ids(), 10030030);
    expect("restore", syscall3(SYS_setresgid, 0, 0, 0), 0);
    expect("restored", ids(), 0);
    return finish();
}
