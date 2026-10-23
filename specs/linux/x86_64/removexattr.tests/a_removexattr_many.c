/* removexattr with many attributes on one file: 24 user.* attributes
 * (small values), removing every other one leaves exactly the others,
 * each with its own value; listxattr then holds just their names (order
 * not checked). Values of a few thousand bytes are removed like small
 * ones, and after everything is removed the list is empty and the same
 * names can be created again. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_listxattr 194
#define SYS_removexattr 197
#define ENODATA 61
#define D "/tmp/a_removexattr_many"
#define F D "/f"
#define N 24

static char name[32], list[1024], val[16], big[2000];

static const char *nm(int i) {
    str_cpy(name, "user.attr");
    a_utoa(name + str_len(name), (u64)i);
    return name;
}

static int in_list(const char *n, i64 len) {
    for (i64 p = 0; p < len; p += (i64)str_len(list + p) + 1)
        if (str_eq(list + p, n)) return 1;
    return 0;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    int ok = 1;
    for (int i = 0; i < N; i++) {
        val[0] = (char)('a' + i);
        if (syscall5(SYS_setxattr, F, nm(i), val, 1, 1) != 0) ok = 0;
    }
    check("set_all", ok, 0);
    ok = 1;
    for (int i = 0; i < N; i += 2)
        if (syscall2(SYS_removexattr, F, nm(i)) != 0) ok = 0;
    check("remove_even", ok, 0);
    int even_gone = 1, odd_kept = 1;
    for (int i = 0; i < N; i++) {
        i64 r = syscall4(SYS_getxattr, F, nm(i), val, sizeof val);
        if (i % 2 == 0 && r != -ENODATA) even_gone = 0;
        if (i % 2 == 1 && (r != 1 || val[0] != (char)('a' + i))) odd_kept = 0;
    }
    check("even_gone", even_gone, 0);
    check("odd_kept", odd_kept, 0);
    i64 len = syscall3(SYS_listxattr, F, list, sizeof list);
    /* 12 names: user.attr1..9 odd (5 x 11 bytes) + 11..23 odd (7 x 12 bytes) */
    expect("list_len", len, 5 * 11 + 7 * 12);
    int listed = 1;
    for (int i = 0; i < N; i++)
        if (in_list(nm(i), len) != (i % 2 == 1)) listed = 0;
    check("list_members", listed, 0);
    for (int i = 0; i < (int)sizeof big; i++) big[i] = (char)i;
    expect("set_big", syscall5(SYS_setxattr, F, "user.big", big, sizeof big, 0), 0);
    expect("remove_big", syscall2(SYS_removexattr, F, "user.big"), 0);
    expect("big_gone", syscall4(SYS_getxattr, F, "user.big", big, sizeof big), -ENODATA);
    ok = 1;
    for (int i = 1; i < N; i += 2)
        if (syscall2(SYS_removexattr, F, nm(i)) != 0) ok = 0;
    check("remove_odd", ok, 0);
    expect("list_empty", syscall3(SYS_listxattr, F, list, sizeof list), 0);
    ok = 1;
    for (int i = 0; i < N; i++)
        if (syscall5(SYS_setxattr, F, nm(i), "z", 1, 1) != 0) ok = 0;
    check("create_again", ok, 0);
    expect("list_full_again", syscall3(SYS_listxattr, F, list, sizeof list), 10 * 11 + 14 * 12);
    expect("cleanup_f", a_unlink(F), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
