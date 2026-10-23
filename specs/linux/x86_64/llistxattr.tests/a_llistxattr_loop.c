/* llistxattr and the symbolic link limit: a path may follow up to 40
 * links in a row. A chain of 40 links (each pointing at the previous one,
 * the first at a directory) used with a trailing slash or as a directory
 * component resolves; with 41 links it is ELOOP. The last link of the
 * chain without a trailing slash is not followed, so it lists as empty
 * whatever the chain's length. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_llistxattr 195
#define ELOOP 40
#define D "/tmp/a_llistxattr_loop"

static char buf[256];
static char name[64], target[64];

static i64 llist(const char *p) { return syscall3(SYS_llistxattr, p, buf, sizeof buf); }

/* D "/cNN" for link number n (1..41); link 1 points at "sub". */
static void link_name(char *out, int n, int full) {
    char *q = full ? str_cpy(out, D "/c") : str_cpy(out, "c");
    q[0] = '0' + n / 10;
    q[1] = '0' + n % 10;
    q[2] = 0;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D "/sub", "user.dir", "x", 1, 0), 0);
    expect("setup_set_file", syscall5(SYS_setxattr, D "/sub/f", "user.f", "x", 1, 0), 0);
    int made = 0;
    for (int n = 1; n <= 41; n++) {
        link_name(name, n, 1);
        if (n == 1) str_cpy(target, "sub");
        else link_name(target, n - 1, 0);
        if (syscall2(SYS_symlink, target, name) == 0) made++;
    }
    expect("setup_links", made, 41);
    expect("c01_self", llist(D "/c01"), 0);
    expect("c01_slash", llist(D "/c01/"), 9);
    expect("c40_self", llist(D "/c40"), 0);
    expect("c40_slash", llist(D "/c40/"), 9);
    check("c40_slash_bytes", mem_eq(buf, "user.dir\0", 9), 0);
    expect("c40_component", llist(D "/c40/f"), 7);
    expect("c41_self", llist(D "/c41"), 0);
    expect("c41_slash", llist(D "/c41/"), -ELOOP);
    expect("c41_component", llist(D "/c41/f"), -ELOOP);
    expect("c41_size0", syscall3(SYS_llistxattr, D "/c41/f", 0, 0), -ELOOP);
    for (int n = 1; n <= 41; n++) {
        link_name(name, n, 1);
        a_unlink(name);
    }
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
