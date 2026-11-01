/* rseq argument checks with nothing registered: len below 32 (0, 16,
 * 31) is EINVAL; an area not aligned to 32 bytes is EINVAL (also with a
 * longer len); any len of 32 or more with a 32-byte aligned area works
 * (33, 64, 4096); an unreadable area (NULL, a low address, an unmapped
 * page, a non-canonical address) is EFAULT; a read-only area is
 * accepted, but the thread is then killed by SIGSEGV on its way back to
 * user mode (the kernel cannot write the CPU number); flags other than 0 and 1 are
 * EINVAL; unregistering when nothing is registered is EINVAL. flags,
 * len and sig are 32-bit: the upper halves of the registers are ignored.
 */
#include "a_sys.h"

#define SYS_rseq 334
#define RSEQ_FLAG_UNREGISTER 1
#define SIG 0x53053053

static char area[8192] __attribute__((aligned(4096)));

static i64 rseq(void *a, u64 len, u64 flags, u64 sig) { return syscall4(SYS_rseq, a, len, flags, sig); }

/* Register, then unregister with the same arguments; 0 if both work. */
static i64 roundtrip(void *a, u64 len) {
    i64 r = rseq(a, len, 0, SIG);
    if (r) return r;
    return rseq(a, len, RSEQ_FLAG_UNREGISTER, SIG) ? -1000 : 0;
}

int test_main(void) {
    expect("len_0_einval", rseq(area, 0, 0, SIG), -EINVAL);
    expect("len_16_einval", rseq(area, 16, 0, SIG), -EINVAL);
    expect("len_31_einval", rseq(area, 31, 0, SIG), -EINVAL);
    expect("misaligned_8_einval", rseq(area + 8, 32, 0, SIG), -EINVAL);
    expect("misaligned_16_einval", rseq(area + 16, 32, 0, SIG), -EINVAL);
    expect("misaligned_16_len64_einval", rseq(area + 16, 64, 0, SIG), -EINVAL);
    expect("flags_2_einval", rseq(area, 32, 2, SIG), -EINVAL);
    expect("flags_minus1_einval", rseq(area, 32, 0xffffffffUL, SIG), -EINVAL);
    expect("unregister_nothing_einval", rseq(area, 32, RSEQ_FLAG_UNREGISTER, SIG), -EINVAL);

    expect("null_efault", rseq(0, 32, 0, SIG), -EFAULT);
    expect("low_address_efault", rseq((void *)32, 32, 0, SIG), -EFAULT);
    expect("non_canonical_efault", rseq((void *)0x800000000000UL, 32, 0, SIG), -EFAULT);
    char *hole = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, hole, A_PAGE);
    expect("unmapped_efault", rseq(hole, 32, 0, SIG), -EFAULT);
    /* A readable but not writable area is accepted, and the thread is
     * killed by SIGSEGV when the kernel then fails to update it. */
    char *ro = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        rseq(ro, 32, 0, SIG);
        syscall1(SYS_exit, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    check("read_only_area_sigsegv", A_WIFSIGNALED(status) && A_WTERMSIG(status) == SIGSEGV, status);

    expect("len_32_ok", roundtrip(area, 32), 0);
    expect("len_33_ok", roundtrip(area, 33), 0);
    expect("len_64_ok", roundtrip(area, 64), 0);
    expect("len_4096_ok", roundtrip(area, 4096), 0);
    expect("aligned_32_offset_ok", roundtrip(area + 32, 64), 0);
    expect("page_end_ok", roundtrip(area + A_PAGE - 32, 32), 0);

    /* Upper halves ignored. */
    expect("len_upper_bits", rseq(area, (1UL << 32) | 32, 0, SIG), 0);
    expect("flags_upper_bits", rseq(area, 32, (1UL << 32) | RSEQ_FLAG_UNREGISTER, SIG), 0);
    expect("sig_upper_bits", rseq(area, 32, 0, (1UL << 32) | SIG), 0);
    expect("sig_upper_bits_same_sig", rseq(area, 32, RSEQ_FLAG_UNREGISTER, SIG), 0);
    return finish();
}
