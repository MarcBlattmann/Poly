/* dup3: replacing an open target closes the file that was there and
 * shares the offset of the source. */
#include "a_sys.h"

#define D "/tmp/a_dup3_replace"

int test_main(void) {
    char c;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_a", a_make_file(D "/a", "AAAA", 0644), 0);
    expect("setup_b", a_make_file(D "/b", "BBBB", 0644), 0);
    i64 a = a_open(D "/a", O_RDONLY, 0);
    i64 b = a_open(D "/b", O_RDONLY, 0);
    expect("open_a_b", a * 10 + b, 34);
    a_read(a, &c, 1);
    expect("dup3_a_onto_b", syscall3(SYS_dup3, a, b, O_CLOEXEC), b);
    expect("b_now_reads_a", a_read(b, &c, 1), 1);
    check("b_now_reads_a_contents", c == 'A', c);
    expect("offset_shared_2", syscall3(SYS_lseek, a, 0, SEEK_CUR), 2);
    expect("b_cloexec", syscall3(SYS_fcntl, b, F_GETFD, 0), FD_CLOEXEC);
    expect("a_not_cloexec", syscall3(SYS_fcntl, a, F_GETFD, 0), 0);
    a_close(a);
    expect("b_survives_close_of_a", a_read(b, &c, 1), 1);
    a_close(b);
    expect("all_closed_next_3", a_open(D "/b", O_RDONLY, 0), 3);
    a_close(3);
    expect("cleanup_a", a_unlink(D "/a"), 0);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
