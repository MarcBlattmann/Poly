/* chroot path length limits: a path of 4096 bytes or more before its NUL
 * is ENAMETOOLONG (36) whatever it contains; 4095 bytes resolves (here to
 * "/", then EPERM). A single name over 255 bytes is ENAMETOOLONG, 255
 * bytes is an ordinary (missing) name. Components are checked left to
 * right as the walk reaches them: a missing directory before a long name
 * gives ENOENT, a file before it ENOTDIR. The test empties its capability
 * sets first, so root and a normal user see the same. */
#include "a_sys.h"

#define SYS_capset 126
#define SYS_chroot 161
#define D "/tmp/a_chroot_name_too_long"

static i64 drop_caps(void) {
    u32 hdr[2] = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    u32 data[6] = { 0, 0, 0, 0, 0, 0 };
    return syscall2(SYS_capset, hdr, data);
}

static char buf[5000];

/* buf = prefix followed by n copies of 'a', NUL-terminated. */
static const char *name(const char *prefix, int n) {
    int i = 0;
    while (prefix[i]) {
        buf[i] = prefix[i];
        i++;
    }
    for (int k = 0; k < n; k++) buf[i++] = 'a';
    buf[i] = 0;
    return buf;
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/file", "x", 0644), 0);

    expect("name_256_enametoolong", syscall1(SYS_chroot, name(D "/", 256)), -ENAMETOOLONG);
    expect("name_255_enoent", syscall1(SYS_chroot, name(D "/", 255)), -ENOENT);
    expect("relative_256_enametoolong", syscall1(SYS_chroot, name("", 256)), -ENAMETOOLONG);
    expect("missing_then_long_enoent", syscall1(SYS_chroot, name(D "/nope/", 256)), -ENOENT);
    expect("file_then_long_enotdir", syscall1(SYS_chroot, name(D "/file/", 256)), -ENOTDIR);

    /* "/" then "./" repeated: 4095 bytes resolves to "/". */
    buf[0] = '/';
    for (int i = 1; i < 4095; i += 2) {
        buf[i] = '.';
        buf[i + 1] = '/';
    }
    buf[4095] = 0;
    expect("path_4095_eperm", syscall1(SYS_chroot, buf), -EPERM);
    buf[4095] = '.';
    buf[4096] = 0;
    expect("path_4096_enametoolong", syscall1(SYS_chroot, buf), -ENAMETOOLONG);
    for (int i = 0; i < 4500; i++) buf[i] = 'x';
    buf[4500] = 0;
    expect("path_4500_enametoolong", syscall1(SYS_chroot, buf), -ENAMETOOLONG);

    expect("cleanup_file", a_unlink(D "/file"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
