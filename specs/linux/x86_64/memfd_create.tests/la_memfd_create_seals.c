/* Sealing: without MFD_ALLOW_SEALING the file starts with F_SEAL_SEAL,
 * so F_GET_SEALS is 1 and F_ADD_SEALS is EPERM. With it the seals start
 * at 0 and can be added: F_SEAL_WRITE makes write EPERM (ftruncate still
 * works), F_SEAL_SHRINK makes shrinking EPERM, F_SEAL_GROW makes growing
 * (ftruncate or a write past the end) EPERM, F_SEAL_SEAL forbids adding
 * more (EPERM); F_GET_SEALS reports the union. A writable MAP_SHARED
 * mapping of a write-sealed file is EPERM; adding F_SEAL_WRITE while
 * such a mapping exists is EBUSY.
 */
#include "a_sys.h"

#define SYS_memfd_create 319
/* linux/memfd.h */
#define MFD_CLOEXEC 0x1
#define MFD_ALLOW_SEALING 0x2
/* linux/fcntl.h */
#define F_ADD_SEALS 1033
#define F_GET_SEALS 1034
#define F_SEAL_SEAL 0x1
#define F_SEAL_SHRINK 0x2
#define F_SEAL_GROW 0x4
#define F_SEAL_WRITE 0x8
#define F_SEAL_FUTURE_WRITE 0x10
#define F_SEAL_EXEC 0x20

static i64 memfd(const char *name, u64 flags) { return syscall2(SYS_memfd_create, name, flags); }
static i64 seals(i64 fd) { return syscall2(SYS_fcntl, fd, F_GET_SEALS); }
static i64 add(i64 fd, i64 s) { return syscall3(SYS_fcntl, fd, F_ADD_SEALS, s); }

int test_main(void) {
    i64 fd = memfd("nosealing", 0);
    expect("default_seal_seal", seals(fd), F_SEAL_SEAL);
    expect("default_add_eperm", add(fd, F_SEAL_WRITE), -EPERM);
    expect("default_still_writable", a_write(fd, "ab", 2), 2);
    a_close(fd);

    fd = memfd("sealing", MFD_ALLOW_SEALING | MFD_CLOEXEC);
    expect("allow_no_seals", seals(fd), 0);
    expect("write_before", a_write(fd, "abcdef", 6), 6);

    expect("add_shrink", add(fd, F_SEAL_SHRINK), 0);
    expect("seals_shrink", seals(fd), F_SEAL_SHRINK);
    expect("shrink_eperm", syscall2(SYS_ftruncate, fd, 3), -EPERM);
    expect("grow_ok", syscall2(SYS_ftruncate, fd, 100), 0);
    expect("same_size_ok", syscall2(SYS_ftruncate, fd, 100), 0);

    expect("add_grow", add(fd, F_SEAL_GROW), 0);
    expect("seals_shrink_grow", seals(fd), F_SEAL_SHRINK | F_SEAL_GROW);
    expect("grow_eperm", syscall2(SYS_ftruncate, fd, 200), -EPERM);
    expect("write_inside_ok", syscall4(SYS_pwrite64, fd, "zz", 2, 98), 2);
    expect("write_past_end_eperm", syscall4(SYS_pwrite64, fd, "zz", 2, 99), -EPERM);

    expect("add_write", add(fd, F_SEAL_WRITE), 0);
    expect("seals_three", seals(fd), F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE);
    expect("write_eperm", syscall4(SYS_pwrite64, fd, "q", 1, 0), -EPERM);
    char c = 0;
    expect("read_ok", syscall4(SYS_pread64, fd, &c, 1, 0), 1);
    expect("read_data", c, 'a');
    i64 m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    expect("shared_writable_map_eperm", m, -EPERM);
    m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_SHARED, fd, 0);
    check("shared_readonly_map_ok", m > 0, m);
    if (m > 0) syscall2(SYS_munmap, m, A_PAGE);
    m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    check("private_writable_map_ok", m > 0, m);
    if (m > 0) syscall2(SYS_munmap, m, A_PAGE);

    expect("add_seal", add(fd, F_SEAL_SEAL), 0);
    expect("seals_four", seals(fd), F_SEAL_SEAL | F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE);
    expect("add_after_seal_eperm", add(fd, F_SEAL_EXEC), -EPERM);
    expect("add_existing_after_seal_eperm", add(fd, F_SEAL_WRITE), -EPERM);
    a_close(fd);

    /* A writable shared mapping blocks F_SEAL_WRITE. */
    fd = memfd("busy", MFD_ALLOW_SEALING);
    syscall2(SYS_ftruncate, fd, A_PAGE);
    m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("map_before_seal", m > 0, m);
    expect("seal_write_with_mapping_ebusy", add(fd, F_SEAL_WRITE), -EBUSY);
    expect("seals_unchanged", seals(fd), 0);
    syscall2(SYS_munmap, m, A_PAGE);
    expect("seal_write_after_unmap", add(fd, F_SEAL_WRITE), 0);
    a_close(fd);

    /* Unknown seal bits are EINVAL. */
    fd = memfd("bits", MFD_ALLOW_SEALING);
    expect("unknown_seal_einval", add(fd, 0x40), -EINVAL);
    expect("future_write", add(fd, F_SEAL_FUTURE_WRITE), 0);
    expect("future_write_blocks_write", a_write(fd, "x", 1), -EPERM);
    expect("seals_future", seals(fd), F_SEAL_FUTURE_WRITE);
    a_close(fd);
    return finish();
}
