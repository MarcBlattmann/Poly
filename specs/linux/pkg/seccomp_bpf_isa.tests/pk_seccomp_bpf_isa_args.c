/* Filters that decide on arguments, the way sandboxes write them. A filter
 * sees each of the six arguments as the full 64-bit register value, as two
 * 32-bit words (low half at 16 + 8*i, high half at 20 + 8*i), whatever
 * type the system call gives the argument: a 32-bit int passed with
 * garbage in the upper half shows the garbage; -1 shows as two words of
 * 0xffffffff; a sign-extended -2 has the high word 0xffffffff. A rule that
 * looks only at the low word matches calls that carry anything above it.
 * Typical filters: check the architecture first (kill on anything but
 * x86-64), deny a few call numbers with ERRNO, deny a call when a flag bit
 * is set in the low word (JSET), or when a 64-bit argument equals a value
 * (two comparisons).
 * The probed call is sched_yield, which ignores its arguments and returns 0.
 * poly-diff: syscalls=off (every case forks)
 */
#include "p_seccomp.h"

#define CLONE_NEWUSER_ 0x10000000U

static i64 probe(const struct pk_insn *f, u16 n, i64 a0, i64 a1) {
    struct pk_result r;
    pk_try(&r, f, n, SYS_sched_yield, a0, a1, 0, 0);
    if (r.load != 0) return 100000 + r.load;
    if (!r.ran) return 200000 + (A_WIFSIGNALED(r.status) ? A_WTERMSIG(r.status) : 0);
    return -r.ret; /* 0 when the call ran */
}

/* deny with EPERM (1) when argument 0 equals the 64-bit value `v` (both halves compared) */
static i64 eq64(i64 v, i64 arg) {
    struct pk_insn f[] = {PK_LDABS(SD_ARG_LO(0)), PK_JEQ((u32)v, 0, 3), PK_LDABS(SD_ARG_HI(0)), PK_JEQ((u32)((u64)v >> 32), 0, 1),
                          PK_ERR(1), PK_ALLOW};
    return probe(f, PK_N(f), arg, 0);
}

/* the same, looking only at the low word (the mistake) */
static i64 eq_low_only(u32 v, i64 arg) {
    struct pk_insn f[] = {PK_LDABS(SD_ARG_LO(0)), PK_JEQ(v, 0, 1), PK_ERR(1), PK_ALLOW};
    return probe(f, PK_N(f), arg, 0);
}

int test_main(void) {
    a_block_sigchld();
    expect("eq64_match", eq64(0x0000000100000002L, 0x0000000100000002L), 1);
    expect("eq64_low_differs", eq64(0x0000000100000002L, 0x0000000100000003L), 0);
    expect("eq64_high_differs", eq64(0x0000000100000002L, 0x0000000200000002L), 0);
    expect("eq64_low_only_matches", eq64(2, 2), 1);
    expect("eq64_low_only_high_garbage", eq64(2, 0x0000000100000002L), 0);
    expect("low_only_rule_sees_through_garbage", eq_low_only(2, 0x0000000100000002L), 1);
    expect("low_only_rule_plain", eq_low_only(2, 2), 1);
    expect("low_only_rule_other", eq_low_only(2, 3), 0);

    /* -1 and sign extension */
    {
        struct pk_insn f[] = {PK_LDABS(SD_ARG_LO(0)), PK_JEQ(0xffffffffU, 0, 3), PK_LDABS(SD_ARG_HI(0)), PK_JEQ(0xffffffffU, 0, 1),
                              PK_ERR(1), PK_ALLOW};
        expect("minus_one_is_all_ones", probe(f, PK_N(f), -1, 0), 1);
        expect("minus_one_as_u32_has_zero_high_word", probe(f, PK_N(f), 0xffffffffL, 0), 0);
        struct pk_insn g[] = {PK_LDABS(SD_ARG_HI(0)), PK_JEQ(0xffffffffU, 0, 1), PK_ERR(2), PK_ALLOW};
        expect("minus_two_high_word_all_ones", probe(g, PK_N(g), -2, 0), 2);
        expect("two_high_word_zero", probe(g, PK_N(g), 2, 0), 0);
    }
    /* flag test with JSET on the low word, as for CLONE_NEWUSER */
    {
        struct pk_insn f[] = {PK_LDABS(SD_ARG_LO(0)), PK_JUMP(BPF_JMP | BPF_JSET | BPF_K, CLONE_NEWUSER_, 0, 1), PK_ERR(1), PK_ALLOW};
        expect("jset_flag_set", probe(f, PK_N(f), CLONE_NEWUSER_ | 17, 0), 1);
        expect("jset_flag_clear", probe(f, PK_N(f), 17, 0), 0);
        expect("jset_flag_in_high_word_not_seen", probe(f, PK_N(f), (i64)((u64)CLONE_NEWUSER_ << 32) | 17, 0), 0);
    }
    /* two different arguments in one rule */
    {
        struct pk_insn f[] = {PK_LDABS(SD_ARG_LO(1)), PK_JEQ(0x5412, 0, 3), PK_LDABS(SD_ARG_LO(0)), PK_JEQ(7, 0, 1), PK_ERR(25),
                              PK_ALLOW};
        expect("rule_on_arg0_and_arg1_hit", probe(f, PK_N(f), 7, 0x5412), 25);
        expect("rule_on_arg0_and_arg1_miss0", probe(f, PK_N(f), 8, 0x5412), 0);
        expect("rule_on_arg0_and_arg1_miss1", probe(f, PK_N(f), 7, 0x5401), 0);
    }
    /* architecture check first, then a deny list by number */
    {
        struct pk_insn f[] = {
            PK_LDABS(SD_ARCH),
            PK_JEQ(AUDIT_ARCH_X86_64, 1, 0),
            PK_RET(SECCOMP_RET_KILL_PROCESS),
            PK_LDABS(SD_NR),
            PK_JEQ(SYS_getuid, 0, 1),
            PK_ERR(1),
            PK_JEQ(SYS_sched_yield, 0, 1),
            PK_ERR(13),
            PK_ALLOW,
        };
        expect("arch_ok_denied_number", probe(f, PK_N(f), 0, 0), 13);
        f[1].k = 0x40000003U; /* AUDIT_ARCH_I386: our native calls do not match */
        struct pk_result r;
        pk_try(&r, f, PK_N(f), SYS_sched_yield, 0, 0, 0, 0);
        check("arch_mismatch_kills", r.load == 0 && !r.ran && pk_killed_by(&r, SIGSYS), r.status);
    }
    return finish();
}
