/* fchmod: only the low 12 bits of the mode argument count. File type
 * bits and anything above 07777 (also above 16 bits, and the upper half
 * of the 64-bit register) are ignored, so a mode of -1 means 07777. */
#include "a_sys.h"

#define F "/tmp/c_fchmod_mode_bits"

static i64 fmode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    a_unlink(F);
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_EXCL, 0600);
    check("setup_open", fd >= 0, fd);
    expect("type_dir_ignored", syscall2(SYS_fchmod, fd, S_IFDIR | 0640), 0);
    expect("still_regular_0640", fmode(fd), S_IFREG | 0640);
    expect("type_lnk_ignored", syscall2(SYS_fchmod, fd, S_IFLNK | 0604), 0);
    expect("still_regular_0604", fmode(fd), S_IFREG | 0604);
    expect("type_fmt_ignored", syscall2(SYS_fchmod, fd, S_IFMT | 0444), 0);
    expect("still_regular_0444", fmode(fd), S_IFREG | 0444);
    expect("bit16_ignored", syscall2(SYS_fchmod, fd, 0x10000 | 0640), 0);
    expect("mode_0640", fmode(fd), S_IFREG | 0640);
    expect("high_word_ignored", syscall2(SYS_fchmod, fd, 0xffff0000L | 0600), 0);
    expect("mode_0600", fmode(fd), S_IFREG | 0600);
    expect("upper_register_ignored", syscall2(SYS_fchmod, fd, (i64)0x7654321000000000L | 0751), 0);
    expect("mode_0751", fmode(fd), S_IFREG | 0751);
    expect("minus_one", syscall2(SYS_fchmod, fd, -1), 0);
    expect("minus_one_is_07777", fmode(fd), S_IFREG | 07777);
    expect("type_bits_only", syscall2(SYS_fchmod, fd, S_IFREG), 0);
    expect("type_bits_only_is_0", fmode(fd), S_IFREG);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
