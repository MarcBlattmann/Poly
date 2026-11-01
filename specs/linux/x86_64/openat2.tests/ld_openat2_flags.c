/* openat2 validates struct open_how strictly, unlike openat:
 *   - flags: any bit that is not a known O_* flag (also bits 32..63):
 *     EINVAL;
 *   - mode: must be 0 unless O_CREAT or O_TMPFILE is given; with them,
 *     bits outside 07777 are EINVAL;
 *   - O_PATH only with O_DIRECTORY, O_NOFOLLOW, O_CLOEXEC (anything else,
 *     also O_LARGEFILE or an access mode, EINVAL);
 *   - O_TMPFILE needs O_WRONLY or O_RDWR; O_DIRECTORY | O_CREAT EINVAL;
 *   - resolve: unknown bits EINVAL; RESOLVE_BENEATH with RESOLVE_IN_ROOT
 *     EINVAL; RESOLVE_CACHED with O_CREAT, O_TRUNC or O_TMPFILE EAGAIN.
 * All of this before the path is read and before dirfd is looked at.
 * Known but harmless flags (O_NOCTTY, FASYNC, O_LARGEFILE, access mode 3)
 * are accepted.
 */
#include "a_sys.h"

#define SYS_openat2 437
#define DIR "/tmp/ld_openat2_flags"

/* linux/openat2.h */
struct open_how {
    u64 flags, mode, resolve;
};
#define RESOLVE_NO_XDEV 0x01
#define RESOLVE_NO_MAGICLINKS 0x02
#define RESOLVE_NO_SYMLINKS 0x04
#define RESOLVE_BENEATH 0x08
#define RESOLVE_IN_ROOT 0x10
#define RESOLVE_CACHED 0x20
#define FASYNC 020000 /* asm-generic/fcntl.h */
static struct open_how how;

static i64 o2(i64 d, const char *p, u64 flags, u64 mode, u64 resolve) {
    how.flags = flags;
    how.mode = mode;
    how.resolve = resolve;
    return syscall4(SYS_openat2, d, p, &how, sizeof how);
}

static void good(const char *name, i64 fd) {
    expect(name, fd, 4);
    if (fd >= 0) a_close(fd);
}

int test_main(void) {
    a_unlink(DIR "/f");
    a_unlink(DIR "/c");
    a_rmdir(DIR);
    expect("mkdir", a_mkdir(DIR, 0755), 0);
    expect("file", a_make_file(DIR "/f", "x", 0644), 0);
    i64 d = a_open(DIR, O_RDONLY | O_DIRECTORY, 0);
    expect("dirfd_3", d, 3);

    expect("flag_bit_23_einval", o2(d, "f", 1UL << 23, 0, 0), -EINVAL);
    expect("flag_bit_31_einval", o2(d, "f", 1UL << 31, 0, 0), -EINVAL);
    expect("flag_bit_32_einval", o2(d, "f", 1UL << 32, 0, 0), -EINVAL);
    expect("flag_bit_63_einval", o2(d, "f", 1UL << 63, 0, 0), -EINVAL);
    good("o_noctty_ok", o2(d, "f", O_NOCTTY, 0, 0));
    good("fasync_ok", o2(d, "f", FASYNC, 0, 0));
    good("o_largefile_ok", o2(d, "f", O_LARGEFILE, 0, 0));
    good("accmode_3_ok", o2(d, "f", O_ACCMODE, 0, 0));

    expect("mode_without_creat_einval", o2(d, "f", O_RDONLY, 0644, 0), -EINVAL);
    expect("mode_1_without_creat_einval", o2(d, "f", O_RDONLY, 1, 0), -EINVAL);
    expect("mode_010000_einval", o2(d, "c", O_CREAT | O_WRONLY, 010000, 0), -EINVAL);
    expect("mode_bit_32_einval", o2(d, "c", O_CREAT | O_WRONLY, (1UL << 32) | 0644, 0), -EINVAL);
    struct a_stat st;
    expect("c_not_created", syscall4(SYS_newfstatat, d, "c", &st, 0), -ENOENT);
    good("mode_07777_creat_ok", o2(d, "c", O_CREAT | O_WRONLY, 07777, 0));

    expect("opath_rdwr_einval", o2(d, "f", O_PATH | O_RDWR, 0, 0), -EINVAL);
    expect("opath_wronly_einval", o2(d, "f", O_PATH | O_WRONLY, 0, 0), -EINVAL);
    expect("opath_nonblock_einval", o2(d, "f", O_PATH | O_NONBLOCK, 0, 0), -EINVAL);
    expect("opath_creat_einval", o2(d, "f", O_PATH | O_CREAT, 0, 0), -EINVAL);
    expect("opath_largefile_einval", o2(d, "f", O_PATH | O_LARGEFILE, 0, 0), -EINVAL);
    expect("opath_trunc_einval", o2(d, "f", O_PATH | O_TRUNC, 0, 0), -EINVAL);
    good("opath_cloexec_nofollow_ok", o2(d, "f", O_PATH | O_CLOEXEC | O_NOFOLLOW, 0, 0));
    expect("tmpfile_rdonly_einval", o2(d, ".", O_TMPFILE | O_RDONLY, 0600, 0), -EINVAL);
    expect("directory_creat_einval", o2(d, "nd", O_DIRECTORY | O_CREAT | O_RDONLY, 0600, 0), -EINVAL);

    expect("resolve_0x40_einval", o2(d, "f", O_RDONLY, 0, 0x40), -EINVAL);
    expect("resolve_bit_63_einval", o2(d, "f", O_RDONLY, 0, 1UL << 63), -EINVAL);
    expect("beneath_in_root_einval", o2(d, "f", O_RDONLY, 0, RESOLVE_BENEATH | RESOLVE_IN_ROOT), -EINVAL);
    expect("cached_creat_eagain", o2(d, "f", O_RDONLY | O_CREAT, 0600, RESOLVE_CACHED), -EAGAIN);
    expect("cached_trunc_eagain", o2(d, "f", O_WRONLY | O_TRUNC, 0, RESOLVE_CACHED), -EAGAIN);
    expect("cached_tmpfile_eagain", o2(d, ".", O_TMPFILE | O_RDWR, 0600, RESOLVE_CACHED), -EAGAIN);
    good("all_four_plain_resolve_bits_ok",
         o2(d, "f", O_RDONLY, 0, RESOLVE_NO_XDEV | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS | RESOLVE_BENEATH));

    /* Validation comes before the path and the descriptor. */
    expect("bad_flags_null_path_einval", o2(d, 0, 1UL << 40, 0, 0), -EINVAL);
    expect("bad_flags_bad_dirfd_einval", o2(99, "f", 1UL << 40, 0, 0), -EINVAL);
    expect("bad_mode_missing_file_einval", o2(d, "nope", O_RDONLY, 0600, 0), -EINVAL);
    expect("bad_resolve_bad_dirfd_einval", o2(99, "f", O_RDONLY, 0, 0x80), -EINVAL);
    expect("cached_creat_bad_dirfd_eagain", o2(99, "f", O_CREAT | O_WRONLY, 0, RESOLVE_CACHED), -EAGAIN);
    expect("null_path_efault", o2(d, 0, O_RDONLY, 0, 0), -EFAULT);
    expect("null_path_bad_dirfd_efault", o2(99, 0, O_RDONLY, 0, 0), -EFAULT);

    a_close(d);
    a_unlink(DIR "/f");
    a_unlink(DIR "/c");
    a_rmdir(DIR);
    return finish();
}
