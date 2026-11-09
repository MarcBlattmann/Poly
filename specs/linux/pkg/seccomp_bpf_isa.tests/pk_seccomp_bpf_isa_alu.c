/* Classic BPF arithmetic in a seccomp filter. The accumulator A and index
 * register X are 32 bits wide; ADD, SUB and MUL wrap around, DIV is
 * unsigned, AND/OR/XOR/LSH/RSH/NEG work as expected, with a constant (K)
 * or the index register (X) as the second operand. A shift by the index
 * register uses only its low 5 bits (a count of 32 shifts by 0). MOD is
 * not part of the language a filter may use (see the reject test). A
 * division by an index register that is 0 at run time kills the thread
 * with SIGSYS (the filter's result is 0, SECCOMP_RET_KILL_THREAD).
 *
 * Each case installs a short filter in a child that computes
 * `A = a; A op= operand; A &= 0xfff; return ERRNO | A` for one probed
 * call (sched_yield), so the call's result -A shows the value.
 * poly-diff: syscalls=off (every case forks)
 */
#include "p_seccomp.h"

static i64 probe(const struct pk_insn *f, u16 n) {
    struct pk_result r;
    pk_try(&r, f, n, SYS_sched_yield, 0, 0, 0, 0);
    if (r.load != 0) return 100000 + r.load;
    if (!r.ran) return 200000 + (A_WIFSIGNALED(r.status) ? A_WTERMSIG(r.status) : 0);
    return -r.ret;
}

/* A = a; (X = operand;) A op= operand; return ERRNO | (A & 0xfff). Returns the value. */
static i64 alu(u32 a, u16 op, int use_x, u32 operand) {
    struct pk_insn f[6];
    int n = 0;
    f[n++] = (struct pk_insn)PK_STMT(BPF_LD | BPF_IMM, a);
    if (use_x) {
        f[n++] = (struct pk_insn)PK_STMT(BPF_LDX | BPF_IMM, operand);
        f[n++] = (struct pk_insn)PK_STMT(BPF_ALU | op | BPF_X, 0);
    } else {
        f[n++] = (struct pk_insn)PK_STMT(BPF_ALU | op | BPF_K, operand);
    }
    f[n++] = (struct pk_insn)PK_STMT(BPF_ALU | BPF_AND | BPF_K, 0xfff);
    f[n++] = (struct pk_insn)PK_STMT(BPF_ALU | BPF_OR | BPF_K, SECCOMP_RET_ERRNO);
    f[n++] = (struct pk_insn)PK_STMT(BPF_RET | BPF_A, 0);
    return probe(f, (u16)n);
}

static void both(const char *name, u32 a, u16 op, u32 operand, i64 want) {
    char nk[48], nx[48];
    str_cat2(nk, name, "_k");
    str_cat2(nx, name, "_x");
    expect(nk, alu(a, op, 0, operand), want);
    expect(nx, alu(a, op, 1, operand), want);
}

int test_main(void) {
    a_block_sigchld();
    both("add", 100, BPF_ADD, 23, 123);
    both("add_wrap", 0xffffffffU, BPF_ADD, 2, 1);
    both("sub", 100, BPF_SUB, 23, 77);
    both("sub_wrap", 1, BPF_SUB, 2, 4095); /* 0xffffffff & 0xfff */
    both("mul", 100, BPF_MUL, 23, 2300);
    both("mul_wrap", 0x10001, BPF_MUL, 0x10001, 1); /* 0x100020001 -> 0x20001 */
    both("div", 100, BPF_DIV, 7, 14);
    both("div_unsigned", 0x80000000U, BPF_DIV, 3, 0xaaa); /* 0x2aaaaaaa & 0xfff */
    both("div_by_larger", 5, BPF_DIV, 9, 0);
    both("or", 0x0f0, BPF_OR, 0x00f, 255);
    both("and", 0x0ff, BPF_AND, 0x03c, 60);
    both("xor", 0x0ff, BPF_XOR, 0x03c, 195);
    both("lsh", 3, BPF_LSH, 4, 48);
    both("rsh", 0x300, BPF_RSH, 4, 48);
    both("lsh_31", 1, BPF_LSH, 31, 0); /* 0x80000000 & 0xfff */
    both("rsh_31", 0x80000000U, BPF_RSH, 31, 1);
    expect("lsh_x_count_32_is_0", alu(3, BPF_LSH, 1, 32), 3);
    expect("rsh_x_count_33_is_1", alu(0x300, BPF_RSH, 1, 33), 0x180);
    {
        struct pk_insn neg[] = {PK_STMT(BPF_LD | BPF_IMM, 1), PK_STMT(BPF_ALU | BPF_NEG, 0),
                                PK_STMT(BPF_ALU | BPF_AND | BPF_K, 0xfff), PK_STMT(BPF_ALU | BPF_OR | BPF_K, SECCOMP_RET_ERRNO),
                                PK_STMT(BPF_RET | BPF_A, 0)};
        expect("neg_1", probe(neg, PK_N(neg)), 4095);
        neg[0].k = 0;
        expect("neg_0", probe(neg, PK_N(neg)), 0);
        neg[0].k = 0xfffff001U;
        expect("neg_big", probe(neg, PK_N(neg)), 0xfff);
    }
    /* Division by an index register that is 0 at run time: SIGSYS. */
    {
        struct pk_insn dz[] = {PK_STMT(BPF_LD | BPF_IMM, 5), PK_STMT(BPF_LDX | BPF_IMM, 0), PK_STMT(BPF_ALU | BPF_DIV | BPF_X, 0),
                               PK_ALLOW};
        struct pk_result r;
        pk_try(&r, dz, PK_N(dz), SYS_sched_yield, 0, 0, 0, 0);
        expect("div_by_x_zero_loads", r.load, 0);
        check("div_by_x_zero_kills_with_sigsys", pk_killed_by(&r, SIGSYS), r.status);
        /* the same division with a non-zero index register is fine */
        dz[1].k = 5;
        pk_try(&r, dz, PK_N(dz), SYS_sched_yield, 0, 0, 0, 0);
        expect("div_by_x_nonzero_allows", pk_errno_is(&r, 0), 0);
    }
    /* A is 32 bits wide whatever was loaded: a 64-bit argument half goes in unchanged. */
    {
        struct pk_insn f[] = {PK_LDABS(SD_ARG_HI(0)), PK_STMT(BPF_ALU | BPF_RSH | BPF_K, 24), PK_STMT(BPF_ALU | BPF_OR | BPF_K, SECCOMP_RET_ERRNO),
                              PK_STMT(BPF_RET | BPF_A, 0)};
        struct pk_result r;
        pk_try(&r, f, PK_N(f), SYS_sched_yield, 0x7a00000012345678L, 0, 0, 0);
        expect("arg_high_half_shifted", pk_errno_is(&r, 0x7a), 0);
    }
    return finish();
}
