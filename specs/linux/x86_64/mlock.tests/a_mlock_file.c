/* mlock on file mappings and on this program's own stack and data: all
 * succeed, and the data is unchanged. */
#include "a_sys.h"

#define F "/tmp/a_mlock_file"

static char data[2 * A_PAGE] = {'d'};

int test_main(void) {
    char local[64];
    local[0] = 's';
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    a_write(fd, "file data", 9);
    i64 m = syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_SHARED, fd, 0);
    check("map_file", m > 0, m);
    if (m > 0) {
        expect("mlock_file_map", syscall2(SYS_mlock, m, A_PAGE), 0);
        check("file_map_reads", mem_eq((void *)m, "file data", 9), 0);
        expect("munlock_file_map", syscall2(SYS_munlock, m, A_PAGE), 0);
        syscall2(SYS_munmap, m, A_PAGE);
    }
    a_close(fd);
    expect("mlock_static_data", syscall2(SYS_mlock, (i64)data, sizeof data), 0);
    check("static_data_kept", data[0] == 'd', data[0]);
    expect("mlock_stack", syscall2(SYS_mlock, (i64)local, sizeof local), 0);
    check("stack_kept", local[0] == 's', local[0]);
    expect("munlock_stack", syscall2(SYS_munlock, (i64)local, sizeof local), 0);
    expect("munlock_static", syscall2(SYS_munlock, (i64)data, sizeof data), 0);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
