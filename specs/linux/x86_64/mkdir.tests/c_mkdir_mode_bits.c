/* mkdir's mode argument: the 9 permission bits less the umask, plus the
 * sticky bit (01000) as given; set-user-ID (04000) and set-group-ID
 * (02000) in the argument are dropped (outside a set-group-ID parent),
 * file type bits are ignored, and only the low 16 bits of the argument
 * count: 0x10000|0777 is 0777, -1 is 0xffff, i.e. 07777. The umask never
 * removes the sticky bit. Mode 0 gives a directory with no permission
 * bits. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_mode_bits"

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    expect("setup", a_mkdir(D, 0755), 0);
    expect("mk_0700", mk(D "/a", 0700), 0);
    expect("mode_0700", mode_of(D "/a"), S_IFDIR | 0700);
    expect("mk_0", mk(D "/b", 0), 0);
    expect("mode_0", mode_of(D "/b"), S_IFDIR);
    expect("mk_sticky", mk(D "/c", 01777), 0);
    expect("sticky_kept", mode_of(D "/c"), S_IFDIR | 01755);
    expect("mk_suid", mk(D "/d", 04777), 0);
    expect("suid_dropped", mode_of(D "/d"), S_IFDIR | 0755);
    expect("mk_sgid", mk(D "/e", 02777), 0);
    expect("sgid_dropped", mode_of(D "/e"), S_IFDIR | 0755);
    expect("mk_07777", mk(D "/f", 07777), 0);
    expect("only_sticky_of_07777", mode_of(D "/f"), S_IFDIR | 01755);
    expect("mk_ifreg_bits", mk(D "/g", S_IFREG | 0750), 0);
    expect("type_bits_ignored", mode_of(D "/g"), S_IFDIR | 0750);
    expect("mk_iflnk_bits", mk(D "/h", S_IFLNK | 0711), 0);
    expect("iflnk_ignored", mode_of(D "/h"), S_IFDIR | 0711);
    expect("mk_ffff", mk(D "/i", 0xffff), 0);
    expect("ffff_is_01755", mode_of(D "/i"), S_IFDIR | 01755);
    expect("mk_bit16", mk(D "/j", 0x10000 | 0777), 0);
    expect("bit16_ignored", mode_of(D "/j"), S_IFDIR | 0755);
    expect("mk_high_bits", mk(D "/k", (i64)0xffffffffffff0000UL | 0700), 0);
    expect("high_bits_ignored", mode_of(D "/k"), S_IFDIR | 0700);
    expect("mk_minus_1", mk(D "/l", -1), 0);
    expect("minus_1_is_01755", mode_of(D "/l"), S_IFDIR | 01755);
    expect("mk_0644", mk(D "/m", 0644), 0);
    expect("mode_0644_no_x", mode_of(D "/m"), S_IFDIR | 0644);

    expect("rm_a", a_rmdir(D "/a"), 0);
    expect("rm_b_mode_0", a_rmdir(D "/b"), 0);
    expect("rm_c", a_rmdir(D "/c"), 0);
    expect("rm_d", a_rmdir(D "/d"), 0);
    expect("rm_e", a_rmdir(D "/e"), 0);
    expect("rm_f", a_rmdir(D "/f"), 0);
    expect("rm_g", a_rmdir(D "/g"), 0);
    expect("rm_h", a_rmdir(D "/h"), 0);
    expect("rm_i", a_rmdir(D "/i"), 0);
    expect("rm_j", a_rmdir(D "/j"), 0);
    expect("rm_k", a_rmdir(D "/k"), 0);
    expect("rm_l", a_rmdir(D "/l"), 0);
    expect("rm_m", a_rmdir(D "/m"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
