/* A memfd is shared memory: a MAP_SHARED mapping and read/write see the
 * same bytes; a second MAP_SHARED mapping of the same descriptor sees
 * the first one's stores; a fork child that writes through its
 * descriptor changes the parent's view (one file, not copied); mapping
 * beyond the end of the file and touching it raises SIGBUS (not
 * tested); a dup'd descriptor shares the offset. The file lives until
 * its last descriptor and mapping are gone.
 */
#include "a_sys.h"

#define SYS_memfd_create 319

static i64 memfd(const char *name, u64 flags) { return syscall2(SYS_memfd_create, name, flags); }

int test_main(void) {
    i64 fd = memfd("shared", 0);
    expect("ftruncate", syscall2(SYS_ftruncate, fd, 2 * A_PAGE), 0);
    char *a = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    char *b = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("map_a", (i64)a > 0, (i64)a);
    check("map_b", (i64)b > 0, (i64)b);
    a[0] = 'M';
    a[A_PAGE + 5] = 'N';
    expect("second_map_sees_store", b[0], 'M');
    expect("second_map_second_page", b[A_PAGE + 5], 'N');
    char c = 0;
    syscall4(SYS_pread64, fd, &c, 1, A_PAGE + 5);
    expect("read_sees_store", c, 'N');
    syscall4(SYS_pwrite64, fd, "W", 1, 7);
    expect("map_sees_write", a[7], 'W');

    /* A fork child writes through the descriptor. */
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall4(SYS_pwrite64, fd, "C", 1, 100);
        a[200] = 'D';
        syscall1(SYS_exit, 0);
    }
    i32 status = -1;
    expect("child_waited", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_status", status, 0);
    expect("parent_sees_child_write", a[100], 'C');
    expect("parent_sees_child_store", a[200], 'D');

    /* dup shares the offset. */
    i64 d = syscall1(SYS_dup, fd);
    expect("lseek_set", syscall3(SYS_lseek, fd, 10, SEEK_SET), 10);
    expect("dup_offset_shared", syscall3(SYS_lseek, d, 0, SEEK_CUR), 10);

    /* The mapping keeps the file alive after both descriptors close. */
    a_close(fd);
    a_close(d);
    expect("mapping_survives_close", b[0], 'M');
    a[1] = 'X';
    expect("mappings_still_shared", b[1], 'X');
    syscall2(SYS_munmap, a, 2 * A_PAGE);
    syscall2(SYS_munmap, b, 2 * A_PAGE);
    return finish();
}
