/* The uid and gid arguments are 32-bit values: the upper half of each
 * register is ignored. 0xffffffff and a 64-bit -1 both mean "unchanged";
 * (1 << 32) | id means id; 0x1ffffffff means unchanged. Checked on the
 * caller's own file with the caller's own ids, so it holds for any user. */
#include "a_sys.h"

#define F "/tmp/c_fchown_id_width"

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)(st.st_mode & 07777);
}

int test_main(void) {
    struct a_stat st;
    i64 uid = syscall0(SYS_geteuid), gid = syscall0(SYS_getegid);
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_EXCL, 0644);
    check("setup_open", fd >= 0, fd);
    expect("both_32bit_minus_one", syscall3(SYS_fchown, fd, 0xffffffffL, 0xffffffffL), 0);
    expect("both_64bit_minus_one", syscall3(SYS_fchown, fd, -1L, -1L), 0);
    expect("upper_half_own_ids", syscall3(SYS_fchown, fd, (1L << 32) | uid, (1L << 32) | gid), 0);
    expect("upper_half_all_ones", syscall3(SYS_fchown, fd, 0xffffffff00000000L | uid, 0xffffffff00000000L | gid), 0);
    expect("minus_one_with_upper_bit", syscall3(SYS_fchown, fd, 0x1ffffffffL, 0x1ffffffffL), 0);
    expect("stat", syscall2(SYS_fstat, fd, &st), 0);
    expect("uid_unchanged", (i64)st.st_uid, uid);
    expect("gid_unchanged", (i64)st.st_gid, gid);
    expect("chmod_06755", syscall2(SYS_fchmod, fd, 06755), 0);
    expect("upper_bit_minus_one_is_success", syscall3(SYS_fchown, fd, 0x1ffffffffL, -1), 0);
    expect("bits_cleared", fmode(fd), 00755);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
