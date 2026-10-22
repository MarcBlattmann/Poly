/* chroot with a path pointer that cannot be read: EFAULT (14) for NULL,
 * a small address, an unmapped page, and a string that runs into an
 * unmapped page before its NUL. A string that ends right before the
 * unmapped page is read normally (and is then EPERM: the test empties
 * its capability sets first, so root and a normal user see the same). */
#include "a_sys.h"

#define SYS_capset 126
#define SYS_chroot 161

static i64 drop_caps(void) {
    u32 hdr[2] = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    u32 data[6] = { 0, 0, 0, 0, 0, 0 };
    return syscall2(SYS_capset, hdr, data);
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("null_efault", syscall1(SYS_chroot, 0), -EFAULT);
    expect("low_address_efault", syscall1(SYS_chroot, 1), -EFAULT);

    char *map = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)map > 0, (i64)map);
    if ((i64)map <= 0) return finish();
    expect("munmap_second", syscall2(SYS_munmap, map + A_PAGE, A_PAGE), 0);
    expect("unmapped_efault", syscall1(SYS_chroot, map + A_PAGE), -EFAULT);

    char *end = map + A_PAGE;
    end[-3] = '/';
    end[-2] = '.';
    end[-1] = 0;
    expect("ends_before_hole_eperm", syscall1(SYS_chroot, end - 3), -EPERM);
    end[-1] = '/';
    expect("runs_into_hole_efault", syscall1(SYS_chroot, end - 3), -EFAULT);
    expect("munmap_first", syscall2(SYS_munmap, map, A_PAGE), 0);
    expect("whole_map_gone_efault", syscall1(SYS_chroot, map), -EFAULT);
    return finish();
}
