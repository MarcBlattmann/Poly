/* /proc descriptors list nothing: /proc/self/status, the /proc/self
 * directory and /proc/meminfo give 0, also with size 0 and with an
 * unmapped buffer. */
#include "a_sys.h"

#define SYS_flistxattr 196

static char buf[64];

static i64 flist(i64 fd) { return syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

int test_main(void) {
    i64 st = a_open("/proc/self/status", O_RDONLY, 0);
    check("setup_status", st >= 0, st);
    expect("status", flist(st), 0);
    expect("status_size0", syscall3(SYS_flistxattr, st, 0, 0), 0);
    expect("status_bad_buffer", syscall3(SYS_flistxattr, st, (void *)8, 100), 0);
    i64 dir = a_open("/proc/self", O_RDONLY | O_DIRECTORY, 0);
    check("setup_dir", dir >= 0, dir);
    expect("self_dir", flist(dir), 0);
    i64 mi = a_open("/proc/meminfo", O_RDONLY, 0);
    check("setup_meminfo", mi >= 0, mi);
    expect("meminfo", flist(mi), 0);
    a_close(mi);
    a_close(dir);
    a_close(st);
    return finish();
}
