/* get_robust_list(pid, head_ptr, len_ptr) stores the robust list head
 * of thread pid (0: the caller) at *head_ptr and 24 at *len_ptr (both
 * 8-byte values) and returns 0. The caller's own pid or tid works like
 * 0. A pid with no such thread (a large unused number, -1) is ESRCH,
 * checked before the pointers (ESRCH even with NULL pointers); an
 * unwritable head_ptr or len_ptr is EFAULT. pid is a 32-bit int: the
 * upper half of the register is ignored.
 */
#include "a_sys.h"

#define SYS_set_robust_list 273
#define SYS_get_robust_list 274

static u64 head, len;
static i64 get(i64 pid) {
    head = 0x5555;
    len = 0x5555;
    return syscall3(SYS_get_robust_list, pid, &head, &len);
}

int test_main(void) {
    syscall2(SYS_set_robust_list, 0xabc0, 24);
    expect("self_0", get(0), 0);
    expect("self_0_head", (i64)head, 0xabc0);
    expect("self_0_len", (i64)len, 24);
    i64 me = syscall0(SYS_getpid);
    expect("own_pid", get(me), 0);
    expect("own_pid_head", (i64)head, 0xabc0);
    expect("own_tid", get(syscall0(SYS_gettid)), 0);
    expect("own_pid_upper_bits", get((1L << 32) | me), 0);
    expect("own_pid_upper_bits_head", (i64)head, 0xabc0);

    expect("unused_pid_esrch", get(0x3ffffff0), -ESRCH);
    expect("minus1_esrch", get(-1), -ESRCH);
    expect("unused_pid_null_ptrs_esrch", syscall3(SYS_get_robust_list, 0x3ffffff0, 0, 0), -ESRCH);
    expect("null_head_ptr_efault", syscall3(SYS_get_robust_list, 0, 0, &len), -EFAULT);
    expect("null_len_ptr_efault", syscall3(SYS_get_robust_list, 0, &head, 0), -EFAULT);
    char *ro = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("read_only_head_ptr_efault", syscall3(SYS_get_robust_list, 0, ro, &len), -EFAULT);
    expect("read_only_len_ptr_efault", syscall3(SYS_get_robust_list, 0, &head, ro), -EFAULT);
    return finish();
}
