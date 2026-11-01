/* getdents(2) with small buffers: a buffer too small for the next record
 * is EINVAL (and the position does not move); otherwise as many whole
 * records as fit are returned; reading one record at a time gives every
 * entry once. */
#include "a_sys.h"

#define D "/tmp/lb_getdents_small"
#define SYS_getdents 78

static unsigned char buf[4096];

int test_main(void) {
    char name[32];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    for (int i = 0; i < 20; i++) {
        str_cpy(str_cpy(name, D "/entry_"), "00");
        name[str_len(name) - 2] = (char)('0' + i / 10);
        name[str_len(name) - 1] = (char)('0' + i % 10);
        a_make_file(name, "", 0644);
    }
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("open", fd >= 0, fd);

    expect("count_0_einval", syscall3(SYS_getdents, fd, buf, 0), -EINVAL);
    expect("count_1_einval", syscall3(SYS_getdents, fd, buf, 1), -EINVAL);
    expect("count_23_einval", syscall3(SYS_getdents, fd, buf, 23), -EINVAL);
    /* "." needs 24 bytes */
    expect("count_24_one_record", syscall3(SYS_getdents, fd, buf, 24), 24);
    expect("first_is_dot_or_dotdot", buf[18] == '.', 1);
    /* the failed calls did not move the position: the next record follows */
    i64 records = 1, bytes = 24;
    for (int guard = 0; guard < 100; guard++) {
        i64 n = syscall3(SYS_getdents, fd, buf, 32);
        if (n <= 0) {
            expect("end", n, 0);
            break;
        }
        u16 reclen = *(u16 *)(buf + 16);
        if (n != reclen) check("one_record_per_call", 0, n);
        records++;
        bytes += n;
    }
    expect("all_records", records, 22);
    expect("bytes_total", bytes, 2 * 24 + 20 * 32);
    /* an entry_NN record needs 32 bytes: 31 is too small mid-directory too */
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    i64 n = syscall3(SYS_getdents, fd, buf, 48);
    expect("dot_and_dotdot", n, 48);
    expect("entry_needs_32", syscall3(SYS_getdents, fd, buf, 31), -EINVAL);
    expect("entry_32_ok", syscall3(SYS_getdents, fd, buf, 32), 32);
    /* a buffer that holds 2.5 records gets 2 */
    expect("partial_fit", syscall3(SYS_getdents, fd, buf, 80), 64);
    /* a buffer larger than needed gets everything */
    expect("rewind2", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("big_count", syscall3(SYS_getdents, fd, buf, sizeof buf), 2 * 24 + 20 * 32);
    expect("close", a_close(fd), 0);

    for (int i = 0; i < 20; i++) {
        str_cpy(str_cpy(name, D "/entry_"), "00");
        name[str_len(name) - 2] = (char)('0' + i / 10);
        name[str_len(name) - 1] = (char)('0' + i % 10);
        a_unlink(name);
    }
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
