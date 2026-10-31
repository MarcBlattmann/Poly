/* MFD_NOEXEC_SEAL: the file is created with mode 0666 and the seal
 * F_SEAL_EXEC; sealing is allowed (as with MFD_ALLOW_SEALING), so more
 * seals can be added. Its mode can still change except for the execute
 * bits: fchmod adding any x bit is EPERM, fchmod 0600 works. MFD_EXEC:
 * mode 0777, sealing not allowed (F_GET_SEALS = F_SEAL_SEAL) unless
 * MFD_ALLOW_SEALING is given too (then 0). F_SEAL_EXEC added to a
 * sealable 0777 file makes chmod of the x bits EPERM as well.
 * (Observed with the vm.memfd_noexec sysctl at its default 0.)
 */
#include "a_sys.h"

#define SYS_memfd_create 319
/* linux/memfd.h */
#define MFD_ALLOW_SEALING 0x2
#define MFD_NOEXEC_SEAL 0x8
#define MFD_EXEC 0x10
/* linux/fcntl.h */
#define F_ADD_SEALS 1033
#define F_GET_SEALS 1034
#define F_SEAL_SEAL 0x1
#define F_SEAL_WRITE 0x8
#define F_SEAL_EXEC 0x20

static i64 memfd(const char *name, u64 flags) { return syscall2(SYS_memfd_create, name, flags); }
static i64 seals(i64 fd) { return syscall2(SYS_fcntl, fd, F_GET_SEALS); }
static i64 add(i64 fd, i64 s) { return syscall3(SYS_fcntl, fd, F_ADD_SEALS, s); }
static i64 mode(i64 fd) {
    struct a_stat st;
    i64 r = syscall2(SYS_fstat, fd, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    i64 fd = memfd("noexec", MFD_NOEXEC_SEAL);
    expect("noexec_mode_0666", mode(fd), S_IFREG | 0666);
    expect("noexec_seals_exec", seals(fd), F_SEAL_EXEC);
    expect("noexec_chmod_x_eperm", syscall2(SYS_fchmod, fd, 0777), -EPERM);
    expect("noexec_chmod_ux_eperm", syscall2(SYS_fchmod, fd, 0766), -EPERM);
    expect("noexec_chmod_ox_eperm", syscall2(SYS_fchmod, fd, 0667), -EPERM);
    expect("noexec_mode_unchanged", mode(fd), S_IFREG | 0666);
    expect("noexec_chmod_0600_ok", syscall2(SYS_fchmod, fd, 0600), 0);
    expect("noexec_mode_0600", mode(fd), S_IFREG | 0600);
    expect("noexec_can_add_seal", add(fd, F_SEAL_WRITE), 0);
    expect("noexec_seals_both", seals(fd), F_SEAL_EXEC | F_SEAL_WRITE);
    a_close(fd);

    fd = memfd("noexec2", MFD_NOEXEC_SEAL | MFD_ALLOW_SEALING);
    expect("noexec_allow_seals_exec", seals(fd), F_SEAL_EXEC);
    a_close(fd);

    fd = memfd("exec", MFD_EXEC);
    expect("exec_mode_0777", mode(fd), S_IFREG | 0777);
    expect("exec_seals_seal", seals(fd), F_SEAL_SEAL);
    expect("exec_add_eperm", add(fd, F_SEAL_EXEC), -EPERM);
    a_close(fd);

    fd = memfd("exec2", MFD_EXEC | MFD_ALLOW_SEALING);
    expect("exec_allow_mode_0777", mode(fd), S_IFREG | 0777);
    expect("exec_allow_no_seals", seals(fd), 0);
    expect("exec_allow_add_exec", add(fd, F_SEAL_EXEC), 0);
    expect("sealed_exec_chmod_drop_x_eperm", syscall2(SYS_fchmod, fd, 0666), -EPERM);
    expect("sealed_exec_chmod_same_ok", syscall2(SYS_fchmod, fd, 0777), 0);
    expect("sealed_exec_chmod_drop_w_ok", syscall2(SYS_fchmod, fd, 0555), 0);
    expect("sealed_exec_mode", mode(fd), S_IFREG | 0555);
    a_close(fd);

    fd = memfd("plain", MFD_ALLOW_SEALING);
    expect("plain_mode_0777", mode(fd), S_IFREG | 0777);
    expect("plain_seals_0", seals(fd), 0);
    a_close(fd);
    return finish();
}
