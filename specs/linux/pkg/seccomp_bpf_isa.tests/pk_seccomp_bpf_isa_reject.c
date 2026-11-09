/* Which classic BPF instructions SECCOMP_SET_MODE_FILTER accepts. The
 * checks are made when the program is installed, and a rejected program
 * installs nothing: seccomp returns EINVAL (22).
 * Rejected: byte and half-word absolute loads, absolute loads at an
 * offset that is not a multiple of 4 or is 64 or more, indirect loads
 * (BPF_IND), BPF_MSH, LDX with an absolute address, reads of a scratch
 * word that no instruction wrote before (on any path), scratch words 16
 * and up, MOD (by anything), a division by a constant 0, shifts by a
 * constant of 32 or more, NEG with X, a return of X, a half-word immediate,
 * unknown opcodes, jumps beyond the last instruction, and programs that
 * can fall off the end. Accepted at the limits: offset 60, scratch word 15,
 * shifts by 31, a program of 4096 instructions, and the "length" loads.
 * A program with filter == NULL (length 1) is EINVAL; a filter pointer
 * that is set but unreadable is EFAULT.
 * poly-diff: syscalls=off (every case forks)
 */
#include "p_seccomp.h"

/* Result of installing prefix + f in a child: 0 or -errno. */
static i64 load(const struct pk_insn *f, u16 n) {
    struct pk_result r;
    pk_try(&r, f, n, SYS_sched_yield, 0, 0, 0, 0);
    return r.load;
}

#define REJECTED(name, ...)                                     \
    do {                                                        \
        struct pk_insn f_[] = {__VA_ARGS__};                    \
        expect(name, load(f_, PK_N(f_)), -EINVAL);              \
    } while (0)
#define ACCEPTED(name, ...)                                     \
    do {                                                        \
        struct pk_insn f_[] = {__VA_ARGS__};                    \
        expect(name, load(f_, PK_N(f_)), 0);                    \
    } while (0)

static struct pk_insn big[4097];

int test_main(void) {
    a_block_sigchld();
    /* loads */
    REJECTED("ld_byte_abs", PK_STMT(BPF_LD | BPF_B | BPF_ABS, 0), PK_ALLOW);
    REJECTED("ld_half_abs", PK_STMT(BPF_LD | BPF_H | BPF_ABS, 0), PK_ALLOW);
    REJECTED("ld_abs_offset_2", PK_LDABS(2), PK_ALLOW);
    REJECTED("ld_abs_offset_61", PK_LDABS(61), PK_ALLOW);
    REJECTED("ld_abs_offset_64", PK_LDABS(64), PK_ALLOW);
    REJECTED("ld_abs_offset_huge", PK_LDABS(0xfffffffcU), PK_ALLOW);
    ACCEPTED("ld_abs_offset_60", PK_LDABS(60), PK_ALLOW);
    ACCEPTED("ld_abs_offset_0", PK_LDABS(0), PK_ALLOW);
    REJECTED("ld_ind", PK_STMT(BPF_LD | BPF_W | BPF_IND, 0), PK_ALLOW);
    REJECTED("ld_half_ind", PK_STMT(BPF_LD | BPF_H | BPF_IND, 0), PK_ALLOW);
    REJECTED("ldx_msh", PK_STMT(BPF_LDX | BPF_B | BPF_MSH, 0), PK_ALLOW);
    REJECTED("ldx_abs", PK_STMT(BPF_LDX | BPF_W | BPF_ABS, 0), PK_ALLOW);
    REJECTED("ld_half_imm", PK_STMT(BPF_LD | BPF_H | BPF_IMM, 5), PK_ALLOW);
    ACCEPTED("ld_len", PK_STMT(BPF_LD | BPF_W | BPF_LEN, 0), PK_ALLOW);
    ACCEPTED("ldx_len", PK_STMT(BPF_LDX | BPF_W | BPF_LEN, 0), PK_ALLOW);
    ACCEPTED("ld_imm", PK_STMT(BPF_LD | BPF_IMM, 0xffffffffU), PK_ALLOW);
    ACCEPTED("ldx_imm", PK_STMT(BPF_LDX | BPF_IMM, 0xffffffffU), PK_ALLOW);
    /* scratch memory */
    REJECTED("ld_mem_never_written", PK_STMT(BPF_LD | BPF_MEM, 3), PK_ALLOW);
    REJECTED("ldx_mem_never_written", PK_STMT(BPF_LDX | BPF_MEM, 0), PK_ALLOW);
    REJECTED("ld_mem_other_slot_written", PK_STMT(BPF_LD | BPF_IMM, 1), PK_STMT(BPF_ST, 2), PK_STMT(BPF_LD | BPF_MEM, 3), PK_ALLOW);
    REJECTED("ld_mem_written_on_one_path_only", PK_LDABS(0), PK_JEQ(1, 0, 1), PK_STMT(BPF_ST, 4), PK_STMT(BPF_LD | BPF_MEM, 4),
             PK_ALLOW);
    REJECTED("st_slot_16", PK_STMT(BPF_ST, 16), PK_ALLOW);
    REJECTED("stx_slot_16", PK_STMT(BPF_STX, 16), PK_ALLOW);
    REJECTED("ld_mem_slot_16", PK_STMT(BPF_ST, 0), PK_STMT(BPF_LD | BPF_MEM, 16), PK_ALLOW);
    ACCEPTED("st_slot_15", PK_STMT(BPF_ST, 15), PK_STMT(BPF_LD | BPF_MEM, 15), PK_ALLOW);
    ACCEPTED("stx_slot_0", PK_STMT(BPF_STX, 0), PK_STMT(BPF_LDX | BPF_MEM, 0), PK_ALLOW);
    ACCEPTED("ld_mem_written_on_both_paths", PK_LDABS(0), PK_JEQ(1, 1, 0), PK_STMT(BPF_ST, 4), PK_STMT(BPF_ST, 4),
             PK_STMT(BPF_LD | BPF_MEM, 4), PK_ALLOW);
    /* arithmetic */
    REJECTED("mod_k", PK_STMT(BPF_ALU | BPF_MOD | BPF_K, 7), PK_ALLOW);
    REJECTED("mod_x", PK_STMT(BPF_ALU | BPF_MOD | BPF_X, 0), PK_ALLOW);
    REJECTED("div_k_zero", PK_STMT(BPF_ALU | BPF_DIV | BPF_K, 0), PK_ALLOW);
    REJECTED("lsh_k_32", PK_STMT(BPF_ALU | BPF_LSH | BPF_K, 32), PK_ALLOW);
    REJECTED("rsh_k_32", PK_STMT(BPF_ALU | BPF_RSH | BPF_K, 32), PK_ALLOW);
    REJECTED("neg_x", PK_STMT(BPF_ALU | BPF_NEG | BPF_X, 0), PK_ALLOW);
    ACCEPTED("lsh_k_31", PK_STMT(BPF_ALU | BPF_LSH | BPF_K, 31), PK_ALLOW);
    ACCEPTED("rsh_k_31", PK_STMT(BPF_ALU | BPF_RSH | BPF_K, 31), PK_ALLOW);
    ACCEPTED("div_k_one", PK_STMT(BPF_ALU | BPF_DIV | BPF_K, 1), PK_ALLOW);
    ACCEPTED("div_x_unknown", PK_STMT(BPF_ALU | BPF_DIV | BPF_X, 0), PK_ALLOW);
    ACCEPTED("neg", PK_STMT(BPF_ALU | BPF_NEG, 0), PK_ALLOW);
    /* returns */
    REJECTED("ret_x", PK_STMT(BPF_RET | BPF_X, 0));
    ACCEPTED("ret_a", PK_STMT(BPF_RET | BPF_A, 0));
    ACCEPTED("ret_k_any_value", PK_RET(0x12345678U));
    /* opcodes */
    REJECTED("unknown_opcode_0xff", PK_STMT(0xff, 0), PK_ALLOW);
    REJECTED("alu_unknown_op_0xd0", PK_STMT(BPF_ALU | 0xd0, 0), PK_ALLOW);
    REJECTED("misc_unknown_0x40", PK_STMT(BPF_MISC | 0x40, 0), PK_ALLOW);
    REJECTED("jmp_unknown_op_0x50", PK_JUMP(BPF_JMP | 0x50, 0, 0, 0), PK_ALLOW);
    REJECTED("st_with_size_bits", PK_STMT(BPF_ST | BPF_H, 0), PK_ALLOW);
    ACCEPTED("tax", PK_STMT(BPF_MISC | BPF_TAX, 0), PK_ALLOW);
    ACCEPTED("txa", PK_STMT(BPF_MISC | BPF_TXA, 0), PK_ALLOW);
    /* control flow */
    REJECTED("ja_past_end", PK_STMT(BPF_JMP | BPF_JA, 2), PK_ALLOW);
    REJECTED("ja_huge", PK_STMT(BPF_JMP | BPF_JA, 0xffffffffU), PK_ALLOW);
    REJECTED("jt_past_end", PK_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 3, 0), PK_ALLOW);
    REJECTED("jf_past_end", PK_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 0, 3), PK_ALLOW);
    REJECTED("falls_off_the_end", PK_LDABS(0));
    REJECTED("last_is_a_jump", PK_ALLOW, PK_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 0, 0));
    ACCEPTED("jump_to_last_instruction", PK_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 1, 1), PK_ALLOW, PK_ALLOW);
    ACCEPTED("code_after_a_return_is_fine", PK_ALLOW, PK_ALLOW);

    /* length limits (the program is installed as it is: PK_RAW) */
    {
        struct pk_result r;
        for (int i = 0; i < 4096; i++) big[i] = (struct pk_insn)PK_ALLOW;
        pk_try_via(PK_SEC | PK_RAW, &r, big, 4096, SYS_sched_yield, 0, 0, 0, 0);
        expect("len_4096_loads", r.load, 0);
        check("len_4096_runs", r.ran && r.ret == 0, r.ret);
        pk_try_via(PK_SEC | PK_RAW, &r, big, 4097, SYS_sched_yield, 0, 0, 0, 0);
        expect("len_4097_einval", r.load, -EINVAL);
        pk_try_via(PK_SEC | PK_RAW, &r, big, 0, SYS_sched_yield, 0, 0, 0, 0);
        expect("len_0_einval", r.load, -EINVAL);
        for (int i = 0; i < 4095; i++) big[i] = (struct pk_insn)PK_STMT(BPF_LD | BPF_IMM, 0);
        pk_try_via(PK_SEC | PK_RAW, &r, big, 4096, SYS_sched_yield, 0, 0, 0, 0);
        expect("len_4096_of_loads_ending_in_allow", r.load, 0);
    }
    /* the filter pointer */
    {
        struct pk_prog nul = {1, 0};
        struct pk_prog low = {1, (struct pk_insn *)8};
        i64 pid = a_fork_sync();
        if (pid == 0) {
            syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
            volatile i32 *s = a_shared();
            s[0] = (i32)syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &nul);
            s[1] = (i32)syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &low);
            syscall1(SYS_exit_group, 0);
        }
        i32 st = -1;
        syscall4(SYS_wait4, pid, &st, 0, 0);
        expect("filter_pointer_null_einval", a_shared()[0], -EINVAL);
        expect("filter_pointer_unreadable_efault", a_shared()[1], -EFAULT);
    }
    return finish();
}
