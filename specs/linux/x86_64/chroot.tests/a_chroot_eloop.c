/* chroot follows symbolic links, in the last component as well as in the
 * middle. A chain of 40 links to a directory resolves (EPERM after
 * resolution: the test empties its capability sets first); 41 is ELOOP
 * (40), and so are a link to itself and two links pointing at each
 * other, also with a trailing slash or used as a directory. */
#include "a_sys.h"

#define SYS_capset 126
#define SYS_chroot 161
#define D "/tmp/a_chroot_eloop"

static i64 drop_caps(void) {
    u32 hdr[2] = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    u32 data[6] = { 0, 0, 0, 0, 0, 0 };
    return syscall2(SYS_capset, hdr, data);
}

/* buf = prefix followed by the two-digit number n. */
static const char *numbered(char *buf, const char *prefix, int n) {
    int i = 0;
    while (prefix[i]) {
        buf[i] = prefix[i];
        i++;
    }
    buf[i++] = (char)('0' + n / 10);
    buf[i++] = (char)('0' + n % 10);
    buf[i] = 0;
    return buf;
}

int test_main(void) {
    char path[64], target[16];
    int made = 0;
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    /* l00 -> dir, lNN -> l(NN-1): lNN is a chain of NN+1 links. */
    for (int i = 0; i < 42; i++) {
        const char *to = i == 0 ? "dir" : numbered(target, "l", i - 1);
        if (syscall3(SYS_symlinkat, to, AT_FDCWD, numbered(path, D "/l", i)) == 0) made++;
    }
    expect("setup_chain", made, 42);
    expect("setup_self", syscall3(SYS_symlinkat, "self", AT_FDCWD, D "/self"), 0);
    expect("setup_a", syscall3(SYS_symlinkat, "b", AT_FDCWD, D "/a"), 0);
    expect("setup_b", syscall3(SYS_symlinkat, "a", AT_FDCWD, D "/b"), 0);

    expect("one_link_eperm", syscall1(SYS_chroot, D "/l00"), -EPERM);
    expect("chain_40_eperm", syscall1(SYS_chroot, D "/l39"), -EPERM);
    expect("chain_41_eloop", syscall1(SYS_chroot, D "/l40"), -ELOOP);
    expect("chain_42_eloop", syscall1(SYS_chroot, D "/l41"), -ELOOP);
    expect("self_eloop", syscall1(SYS_chroot, D "/self"), -ELOOP);
    expect("self_slash_eloop", syscall1(SYS_chroot, D "/self/"), -ELOOP);
    expect("pair_eloop", syscall1(SYS_chroot, D "/a"), -ELOOP);
    expect("pair_middle_eloop", syscall1(SYS_chroot, D "/a/x"), -ELOOP);
    expect("link_middle_eperm", syscall1(SYS_chroot, D "/l05/."), -EPERM);

    made = 0;
    for (int i = 0; i < 42; i++)
        if (a_unlink(numbered(path, D "/l", i)) == 0) made++;
    expect("cleanup_chain", made, 42);
    expect("cleanup_self", a_unlink(D "/self"), 0);
    expect("cleanup_a", a_unlink(D "/a"), 0);
    expect("cleanup_b", a_unlink(D "/b"), 0);
    expect("cleanup_dir", a_rmdir(D "/dir"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
