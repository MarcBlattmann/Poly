/* chroot of a path that does not resolve: ENOENT (2) for a missing name,
 * the empty string, a dangling symbolic link, a missing directory in the
 * middle of the path (a later ".." does not undo it) and a missing
 * relative name. The path is resolved before the privilege check, so a
 * caller without CAP_SYS_CHROOT sees ENOENT, not EPERM: the test first
 * empties its capability sets so root and a normal user see the same. */
#include "a_sys.h"

#define SYS_capset 126
#define SYS_chroot 161
#define D "/tmp/a_chroot_enoent"

static i64 drop_caps(void) {
    u32 hdr[2] = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    u32 data[6] = { 0, 0, 0, 0, 0, 0 };
    return syscall2(SYS_capset, hdr, data);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_dangling", syscall3(SYS_symlinkat, "nowhere", AT_FDCWD, D "/dangling"), 0);
    expect("setup_to_dangling", syscall3(SYS_symlinkat, "dangling", AT_FDCWD, D "/to_dangling"), 0);

    expect("missing_enoent", syscall1(SYS_chroot, D "/nope"), -ENOENT);
    expect("missing_slash_enoent", syscall1(SYS_chroot, D "/nope/"), -ENOENT);
    expect("empty_enoent", syscall1(SYS_chroot, ""), -ENOENT);
    expect("dangling_enoent", syscall1(SYS_chroot, D "/dangling"), -ENOENT);
    expect("dangling_chain_enoent", syscall1(SYS_chroot, D "/to_dangling"), -ENOENT);
    expect("middle_missing_enoent", syscall1(SYS_chroot, D "/nope/sub"), -ENOENT);
    expect("missing_then_dotdot_enoent", syscall1(SYS_chroot, D "/nope/.."), -ENOENT);
    expect("missing_top_enoent", syscall1(SYS_chroot, "/a_chroot_enoent_nope"), -ENOENT);
    expect("chdir_d", syscall1(SYS_chdir, D), 0);
    expect("relative_missing_enoent", syscall1(SYS_chroot, "nope"), -ENOENT);
    expect("relative_dangling_enoent", syscall1(SYS_chroot, "dangling/"), -ENOENT);
    expect("existing_still_eperm", syscall1(SYS_chroot, "."), -EPERM);

    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_to_dangling", a_unlink(D "/to_dangling"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
