/* getdents(2) with small buffers: a buffer too small for the next record
 * is EINVAL (and the position does not move); otherwise as many whole
 * records as fit are returned; reading one record at a time gives every
 * entry once. The order of the entries is the file system's ("." and ".."
 * come first on tmpfs, anywhere on ext4), so the record sizes are taken
 * from a full listing first.
 * poly-diff: syscalls=names (the counts passed depend on the file
 * system's entry order; only the behaviour is checked) */
#include "a_sys.h"

#define D "/tmp/lb_getdents_small"
#define SYS_getdents 78
#define N 22

static unsigned char buf[4096], full[4096];

/* Records are the same entry: d_ino, d_reclen, the name with its NUL and
 * the type byte agree. Left out: d_off (a cookie) and the padding bytes
 * between the name's NUL and the type byte, which getdents does not write
 * (they keep whatever the buffer held). */
static int same_records(const unsigned char *a, const unsigned char *b, i64 len) {
    for (i64 pos = 0; pos < len;) {
        i64 r = *(const u16 *)(a + pos + 16);
        size_t name = str_len((const char *)a + pos + 18) + 1;
        if (r < 24 || !mem_eq(a + pos, b + pos, 8) || !mem_eq(a + pos + 16, b + pos + 16, 2 + name) ||
            a[pos + r - 1] != b[pos + r - 1])
            return 0;
        pos += r;
    }
    return 1;
}

int test_main(void) {
    char name[32];
    i64 rec[N + 1];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    for (int i = 0; i < 20; i++) {
        str_cpy(str_cpy(name, D "/entry_"), "00");
        name[str_len(name) - 2] = (char)('0' + i / 10);
        name[str_len(name) - 1] = (char)('0' + i % 10);
        a_make_file(name, "", 0644);
    }
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("open", fd >= 0, fd);

    /* a full listing: 2 records of 24 bytes ("." and "..") and 20 of 32
     * ("entry_NN"), in the file system's order */
    i64 total = syscall3(SYS_getdents, fd, full, sizeof full);
    expect("full_listing", total, 2 * 24 + 20 * 32);
    int count = 0, small = 0, large = 0;
    for (i64 pos = 0; pos < total && count < N; count++) {
        rec[count] = *(u16 *)(full + pos + 16);
        small += rec[count] == 24;
        large += rec[count] == 32;
        pos += rec[count];
    }
    expect("full_records", count, N);
    expect("records_of_24", small, 2);
    expect("records_of_32", large, 20);
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);

    expect("count_0_einval", syscall3(SYS_getdents, fd, buf, 0), -EINVAL);
    expect("count_1_einval", syscall3(SYS_getdents, fd, buf, 1), -EINVAL);
    expect("count_first_minus_1_einval", syscall3(SYS_getdents, fd, buf, rec[0] - 1), -EINVAL);
    expect("count_first_one_record", syscall3(SYS_getdents, fd, buf, rec[0]), rec[0]);
    expect("first_record_same", same_records(buf, full, rec[0]), 1);
    /* the failed calls did not move the position, and a failed call in the
     * middle does not either */
    expect("count_second_minus_1_einval", syscall3(SYS_getdents, fd, buf, rec[1] - 1), -EINVAL);
    i64 records = 1, bytes = rec[0], bad = 0;
    for (int guard = 0; guard < 100; guard++) {
        i64 n = syscall3(SYS_getdents, fd, buf, 32);
        if (n <= 0) {
            expect("end", n, 0);
            break;
        }
        if (n != *(u16 *)(buf + 16) || !same_records(buf, full + bytes, n)) bad++;
        records++;
        bytes += n;
    }
    expect("one_record_per_call_in_order", bad, 0);
    expect("all_records", records, N);
    expect("bytes_total", bytes, 2 * 24 + 20 * 32);

    /* a buffer that holds two records and almost a third gets two */
    expect("rewind2", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("partial_fit", syscall3(SYS_getdents, fd, buf, rec[0] + rec[1] + rec[2] - 1), rec[0] + rec[1]);
    expect("next_is_third", syscall3(SYS_getdents, fd, buf, rec[2]), rec[2]);
    expect("third_same", same_records(buf, full + rec[0] + rec[1], rec[2]), 1);
    /* an "entry_NN" record needs 32 bytes wherever it comes */
    int k = 3;
    i64 off = rec[0] + rec[1] + rec[2];
    while (k < N && rec[k] != 32) off += rec[k++];
    expect("seek_to_entry", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("skip_before_entry", off ? syscall3(SYS_getdents, fd, buf, off) : 0, off);
    expect("entry_needs_32", syscall3(SYS_getdents, fd, buf, 31), -EINVAL);
    expect("entry_32_ok", syscall3(SYS_getdents, fd, buf, 32), 32);
    /* a buffer larger than needed gets everything */
    expect("rewind3", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("big_count", syscall3(SYS_getdents, fd, buf, sizeof buf), 2 * 24 + 20 * 32);
    expect("big_same", same_records(buf, full, total), 1);
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
