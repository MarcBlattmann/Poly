/* What a filter can load: the fields of struct seccomp_data with absolute
 * 32-bit loads (system call number at 0, architecture at 4, the 64-bit
 * arguments as low and high halves at 16 + 8*i and 20 + 8*i), the data
 * length (64) with a "length" load into A or X (its k is ignored), an
 * immediate into A or X, the 16 scratch words (ST / STX to store A / X,
 * LD / LDX MEM to load them), TAX and TXA, and the two ways to return
 * (a constant K, or A). Each case runs a filter for one probed call
 * (sched_yield with six chosen argument values) and reads off ERRNO
 * results.
 * poly-diff: syscalls=off (every case forks)
 */
#include "p_seccomp.h"

static const i64 args[4] = {0x11111111aaaaaaaaL, 0x2222222233333333L, (i64)0xf000000000000001UL, 4};

static i64 probe(const struct pk_insn *f, u16 n) {
    struct pk_result r;
    pk_try(&r, f, n, SYS_sched_yield, args[0], args[1], args[2], args[3]);
    if (r.load != 0) return 100000 + r.load;
    if (!r.ran) return 200000 + (A_WIFSIGNALED(r.status) ? A_WTERMSIG(r.status) : 0);
    return -r.ret;
}

/* 1 if the word loaded at `off` equals `want`, else 2 */
static i64 word_is(u32 off, u32 want) {
    struct pk_insn f[] = {PK_LDABS(off), PK_JEQ(want, 0, 1), PK_ERR(1), PK_ERR(2)};
    return probe(f, PK_N(f));
}

/* Return ERRNO | (A & 0xfff) after the instructions `pre`. */
static i64 value_of(const struct pk_insn *pre, u16 n) {
    struct pk_insn f[16];
    for (u16 i = 0; i < n; i++) f[i] = pre[i];
    f[n] = (struct pk_insn)PK_STMT(BPF_ALU | BPF_AND | BPF_K, 0xfff);
    f[n + 1] = (struct pk_insn)PK_STMT(BPF_ALU | BPF_OR | BPF_K, SECCOMP_RET_ERRNO);
    f[n + 2] = (struct pk_insn)PK_STMT(BPF_RET | BPF_A, 0);
    return probe(f, (u16)(n + 3));
}

int test_main(void) {
    a_block_sigchld();
    expect("nr", word_is(SD_NR, SYS_sched_yield), 1);
    expect("arch", word_is(SD_ARCH, AUDIT_ARCH_X86_64), 1);
    expect("arg0_low", word_is(SD_ARG_LO(0), 0xaaaaaaaaU), 1);
    expect("arg0_high", word_is(SD_ARG_HI(0), 0x11111111U), 1);
    expect("arg1_low", word_is(SD_ARG_LO(1), 0x33333333U), 1);
    expect("arg1_high", word_is(SD_ARG_HI(1), 0x22222222U), 1);
    expect("arg2_low", word_is(SD_ARG_LO(2), 1), 1);
    expect("arg2_high", word_is(SD_ARG_HI(2), 0xf0000000U), 1);
    expect("arg3_low", word_is(SD_ARG_LO(3), 4), 1);
    expect("arg3_high", word_is(SD_ARG_HI(3), 0), 1);
    expect("arg4_zero", word_is(SD_ARG_LO(4), 0), 1);
    expect("arg5_high_zero", word_is(SD_ARG_HI(5), 0), 1);
    expect("nr_mismatch_detected", word_is(SD_NR, SYS_getuid), 2);
    {
        /* the highest word a filter may load is at offset 60 */
        struct pk_insn f[] = {PK_LDABS(60), PK_JEQ(0, 0, 1), PK_ERR(1), PK_ERR(2)};
        expect("last_word_at_60", probe(f, PK_N(f)), 1);
    }

    /* data length: A and X, whatever k says */
    {
        struct pk_insn a0[] = {PK_STMT(BPF_LD | BPF_W | BPF_LEN, 0)};
        expect("len_into_a", value_of(a0, 1), 64);
        struct pk_insn a1[] = {PK_STMT(BPF_LD | BPF_W | BPF_LEN, 12345)};
        expect("len_into_a_k_ignored", value_of(a1, 1), 64);
        struct pk_insn x0[] = {PK_STMT(BPF_LDX | BPF_W | BPF_LEN, 7), PK_STMT(BPF_MISC | BPF_TXA, 0)};
        expect("len_into_x", value_of(x0, 2), 64);
    }
    /* immediates and register moves */
    {
        struct pk_insn i0[] = {PK_STMT(BPF_LD | BPF_IMM, 321)};
        expect("imm_into_a", value_of(i0, 1), 321);
        struct pk_insn i1[] = {PK_STMT(BPF_LDX | BPF_IMM, 654), PK_STMT(BPF_MISC | BPF_TXA, 0)};
        expect("imm_into_x_txa", value_of(i1, 2), 654);
        struct pk_insn i2[] = {PK_STMT(BPF_LD | BPF_IMM, 41), PK_STMT(BPF_MISC | BPF_TAX, 0), PK_STMT(BPF_LD | BPF_IMM, 0),
                               PK_STMT(BPF_MISC | BPF_TXA, 0)};
        expect("tax_then_txa", value_of(i2, 4), 41);
        struct pk_insn i3[] = {PK_STMT(BPF_LD | BPF_IMM, 40), PK_STMT(BPF_LDX | BPF_IMM, 2), PK_STMT(BPF_ALU | BPF_ADD | BPF_X, 0)};
        expect("a_and_x_separate", value_of(i3, 3), 42);
    }
    /* scratch memory: sixteen independent words */
    {
        struct pk_insn m0[] = {PK_STMT(BPF_LD | BPF_IMM, 77), PK_STMT(BPF_ST, 0), PK_STMT(BPF_LD | BPF_IMM, 0), PK_STMT(BPF_LD | BPF_MEM, 0)};
        expect("st_ld_slot_0", value_of(m0, 4), 77);
        struct pk_insn m15[] = {PK_STMT(BPF_LD | BPF_IMM, 78), PK_STMT(BPF_ST, 15), PK_STMT(BPF_LD | BPF_IMM, 0), PK_STMT(BPF_LD | BPF_MEM, 15)};
        expect("st_ld_slot_15", value_of(m15, 4), 78);
        struct pk_insn mx[] = {PK_STMT(BPF_LDX | BPF_IMM, 79), PK_STMT(BPF_STX, 7), PK_STMT(BPF_LDX | BPF_IMM, 0), PK_STMT(BPF_LDX | BPF_MEM, 7),
                               PK_STMT(BPF_MISC | BPF_TXA, 0)};
        expect("stx_ldx_slot_7", value_of(mx, 5), 79);
        struct pk_insn sep[] = {PK_STMT(BPF_LD | BPF_IMM, 11), PK_STMT(BPF_ST, 1), PK_STMT(BPF_LD | BPF_IMM, 22), PK_STMT(BPF_ST, 2),
                                PK_STMT(BPF_LD | BPF_MEM, 1)};
        expect("slots_independent", value_of(sep, 5), 11);
        /* a word stored from X is seen by a load into A */
        struct pk_insn cross[] = {PK_STMT(BPF_LDX | BPF_IMM, 99), PK_STMT(BPF_STX, 3), PK_STMT(BPF_LD | BPF_MEM, 3)};
        expect("stx_then_ld_a", value_of(cross, 3), 99);
        /* an argument word kept in scratch memory across other work */
        struct pk_insn keep[] = {PK_LDABS(SD_ARG_LO(3)), PK_STMT(BPF_ST, 5), PK_LDABS(SD_NR), PK_STMT(BPF_LD | BPF_MEM, 5)};
        expect("argument_kept_in_slot", value_of(keep, 4), 4);
    }
    /* returning: a constant, or the accumulator */
    {
        struct pk_insn k[] = {PK_ERR(33)};
        expect("ret_k", probe(k, 1), 33);
        struct pk_insn a[] = {PK_STMT(BPF_LD | BPF_IMM, SECCOMP_RET_ERRNO | 34), PK_STMT(BPF_RET | BPF_A, 0)};
        expect("ret_a", probe(a, 2), 34);
        struct pk_insn t[] = {PK_STMT(BPF_LDX | BPF_IMM, SECCOMP_RET_ERRNO | 35), PK_STMT(BPF_MISC | BPF_TXA, 0), PK_STMT(BPF_RET | BPF_A, 0)};
        expect("ret_a_after_txa", probe(t, 3), 35);
        struct pk_insn z[] = {PK_STMT(BPF_LD | BPF_IMM, SECCOMP_RET_ALLOW), PK_STMT(BPF_RET | BPF_A, 0)};
        expect("ret_a_allow", probe(z, 2), 0);
    }
    return finish();
}
