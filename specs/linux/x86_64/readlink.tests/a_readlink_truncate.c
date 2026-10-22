/* readlink: a buffer shorter than the text gets the first bufsiz bytes
 * and the call returns bufsiz; it is not an error. */
#include "a_sys.h"

#define D "/tmp/a_readlink_truncate"

static i64 rl(const char *p, char *buf, i64 n) { return syscall3(SYS_readlink, p, buf, n); }

int test_main(void) {
    char buf[32];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_link", syscall2(SYS_symlink, "abcdefghijklmnop", D "/l"), 0);
    memset(buf, 'z', sizeof buf);
    expect("size_4", rl(D "/l", buf, 4), 4);
    check("size_4_text", mem_eq(buf, "abcd", 4) && buf[4] == 'z', buf[4]);
    memset(buf, 'z', sizeof buf);
    expect("size_1", rl(D "/l", buf, 1), 1);
    check("size_1_text", buf[0] == 'a' && buf[1] == 'z', buf[1]);
    memset(buf, 'z', sizeof buf);
    expect("size_15", rl(D "/l", buf, 15), 15);
    check("size_15_text", mem_eq(buf, "abcdefghijklmno", 15) && buf[15] == 'z', buf[15]);
    memset(buf, 'z', sizeof buf);
    expect("size_16", rl(D "/l", buf, 16), 16);
    expect("size_17", rl(D "/l", buf, 17), 16);
    check("size_17_untouched", buf[16] == 'z', buf[16]);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
