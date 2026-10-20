/* pread64: descriptors that share an open file do not see each other's
 * pread offsets; pread sees data another descriptor just wrote. */
#include "a_sys.h"

#define F "/tmp/a_pread64_dup"

static i64 pread(i64 fd, void *buf, size_t n, i64 off) { return syscall4(SYS_pread64, fd, buf, n, off); }

int test_main(void) {
    char buf[64];
    i64 w = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    i64 r = a_open(F, O_RDONLY, 0);
    i64 d = syscall1(SYS_dup, r);
    expect("fds", w * 100 + r * 10 + d, 345);
    expect("write", a_write(w, "0123456789abcdef", 16), 16);
    expect("pread_sees_write", pread(r, buf, 4, 12), 4);
    check("pread_sees_write_contents", mem_eq(buf, "cdef", 4), buf[0]);
    expect("pread_on_dup", pread(d, buf, 2, 3), 2);
    expect("shared_offset_still_0", syscall3(SYS_lseek, r, 0, SEEK_CUR), 0);
    expect("dup_offset_still_0", syscall3(SYS_lseek, d, 0, SEEK_CUR), 0);
    expect("writer_offset_16", syscall3(SYS_lseek, w, 0, SEEK_CUR), 16);
    /* Many small preads across the file. */
    int ok = 1;
    for (int i = 0; i < 16; i++) {
        char c;
        ok &= pread(r, &c, 1, 15 - i) == 1 && c == "0123456789abcdef"[15 - i];
    }
    check("byte_by_byte_backwards", ok, 0);
    /* A large offset that fits in 63 bits. */
    expect("seek_far", syscall3(SYS_lseek, w, 0x100000000L, SEEK_SET), 0x100000000L);
    expect("write_far", a_write(w, "Z", 1), 1);
    expect("pread_far", pread(r, buf, 8, 0x100000000L), 1);
    check("pread_far_contents", buf[0] == 'Z', buf[0]);
    expect("pread_in_hole", pread(r, buf, 4, 0x80000000L), 4);
    check("pread_in_hole_zeros", buf[0] == 0 && buf[3] == 0, buf[0]);
    a_close(w);
    a_close(r);
    a_close(d);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
