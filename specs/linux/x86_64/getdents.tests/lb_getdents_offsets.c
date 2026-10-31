/* getdents(2) d_off: each record's d_off is the position just after it;
 * lseek to it resumes with the next record; getdents and getdents64 list
 * the same entries with the same d_ino, d_off and type; the descriptor's
 * position after reading everything is the last d_off.
 * poly-diff: syscalls=names (lseek gets raw d_off values, which are the
 * file system's own; only their behaviour is checked) */
#include "a_sys.h"

#define D "/tmp/lb_getdents_offsets"
#define SYS_getdents 78

static unsigned char b1[4096], b2[4096], b3[4096];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_make_file(D "/alpha", "", 0644), 0);
    expect("setup_b", a_mkdir(D "/beta", 0755), 0);
    expect("setup_c", syscall2(SYS_symlink, "alpha", D "/gamma"), 0);
    expect("setup_d", a_make_file(D "/delta", "", 0644), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    i64 n1 = syscall3(SYS_getdents, fd, b1, sizeof b1);
    check("old_read", n1 > 0, n1);
    i64 endpos = syscall3(SYS_lseek, fd, 0, SEEK_CUR);
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    i64 n2 = syscall3(SYS_getdents64, fd, b2, sizeof b2);
    check("new_read", n2 > 0, n2);

    /* walk both lists side by side */
    i64 p1 = 0, p2 = 0, count = 0, same = 0;
    i64 last_off = 0;
    while (p1 < n1 && p2 < n2) {
        u64 ino1 = *(u64 *)(b1 + p1), off1 = *(u64 *)(b1 + p1 + 8);
        u16 len1 = *(u16 *)(b1 + p1 + 16);
        u8 type1 = b1[p1 + len1 - 1];
        struct a_dirent64 *e = (struct a_dirent64 *)(b2 + p2);
        if (ino1 == e->d_ino && (i64)off1 == e->d_off && type1 == e->d_type &&
            str_eq((const char *)(b1 + p1 + 18), e->d_name))
            same++;
        last_off = (i64)off1;
        count++;
        p1 += len1;
        p2 += e->d_reclen;
    }
    expect("six_entries", count, 6);
    expect("same_entries_both_calls", same, 6);
    expect("both_lists_end_together", p1 == n1 && p2 == n2, 1);
    expect("position_is_last_d_off", endpos, last_off);

    /* resume after the third record */
    i64 pos = 0;
    for (int i = 0; i < 2; i++) pos += *(u16 *)(b1 + pos + 16);
    u64 off3 = *(u64 *)(b1 + pos + 8);
    i64 rest = n1 - pos - *(u16 *)(b1 + pos + 16);
    expect("seek_to_d_off", syscall3(SYS_lseek, fd, (i64)off3, SEEK_SET), (i64)off3);
    i64 n3 = syscall3(SYS_getdents, fd, b3, sizeof b3);
    expect("resumed_length", n3, rest);
    expect("resumed_bytes", mem_eq(b3, b1 + n1 - rest, (size_t)rest), 1);
    expect("close", a_close(fd), 0);

    expect("cleanup_d", a_unlink(D "/delta"), 0);
    expect("cleanup_c", a_unlink(D "/gamma"), 0);
    expect("cleanup_b", a_rmdir(D "/beta"), 0);
    expect("cleanup_a", a_unlink(D "/alpha"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
