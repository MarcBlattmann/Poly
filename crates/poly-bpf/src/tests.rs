use super::code::*;
use super::*;
use alloc::vec;

/// `struct seccomp_data` as 16 words.
const SECCOMP: Rules = Rules { data_words: 16 };

fn ok(insns: &[Insn]) -> Program {
    Program::new(insns, SECCOMP).expect("a valid program")
}

fn err(insns: &[Insn]) -> Error {
    Program::new(insns, SECCOMP).expect_err("an invalid program")
}

fn ret(k: u32) -> Insn {
    Insn::stmt(RET | K, k)
}

/// seccomp(2)'s example shape: check the architecture, kill on a
/// mismatch, return `EPERM` for one call, allow the rest.
fn arch_then_deny(nr: u32) -> Vec<Insn> {
    vec![
        Insn::stmt(LD | W | ABS, 4),
        Insn::jump(JMP | JEQ | K, 0xc000_003e, 1, 0),
        ret(0x8000_0000),
        Insn::stmt(LD | W | ABS, 0),
        Insn::jump(JMP | JEQ | K, nr, 0, 1),
        ret(0x0005_0000 | 1),
        ret(0x7fff_0000),
    ]
}

#[test]
fn a_typical_seccomp_filter_runs() {
    let p = ok(&arch_then_deny(39));
    let mut data = [0u32; 16];
    data[1] = 0xc000_003e;
    data[0] = 39;
    assert_eq!(p.run(&data), 0x0005_0001);
    data[0] = 1;
    assert_eq!(p.run(&data), 0x7fff_0000);
    data[1] = 0x4000_0003;
    assert_eq!(p.run(&data), 0x8000_0000);
    assert_eq!(p.len(), 7);
}

#[test]
fn empty_and_too_long_programs_are_refused() {
    assert_eq!(err(&[]), Error::Empty);
    let long = vec![ret(0); MAXINSNS + 1];
    assert_eq!(err(&long), Error::TooLong);
    let max = vec![ret(7); MAXINSNS];
    assert_eq!(ok(&max).run(&[]), 7);
}

#[test]
fn loads_must_be_aligned_words_inside_the_data() {
    for k in [0, 4, 60] {
        ok(&[Insn::stmt(LD | W | ABS, k), Insn::stmt(RET | A, 0)]);
    }
    assert_eq!(err(&[Insn::stmt(LD | W | ABS, 2), ret(0)]), Error::BadLoad { pc: 0 });
    assert_eq!(err(&[Insn::stmt(LD | W | ABS, 64), ret(0)]), Error::BadLoad { pc: 0 });
    assert_eq!(err(&[ret(0), Insn::stmt(LD | W | ABS, u32::MAX - 3), ret(0)]), Error::BadLoad { pc: 1 });
    assert_eq!(err(&[Insn::stmt(LD | H | ABS, 0), ret(0)]), Error::BadLoad { pc: 0 });
    assert_eq!(err(&[Insn::stmt(LD | B | ABS, 0), ret(0)]), Error::BadLoad { pc: 0 });
    assert_eq!(err(&[Insn::stmt(LD | W | IND, 0), ret(0)]), Error::BadLoad { pc: 0 });
    assert_eq!(err(&[Insn::stmt(LDX | B | MSH, 0), ret(0)]), Error::BadLoad { pc: 0 });
}

#[test]
fn loads_read_the_words_and_the_length() {
    let data: Vec<u32> = (100..116).collect();
    let p = ok(&[Insn::stmt(LD | W | ABS, 60), Insn::stmt(RET | A, 0)]);
    assert_eq!(p.run(&data), 115);
    let p = ok(&[Insn::stmt(LD | W | LEN, 0), Insn::stmt(RET | A, 0)]);
    assert_eq!(p.run(&data), 64);
    let p = ok(&[Insn::stmt(LDX | W | LEN, 0), Insn::stmt(RET | X, 0)]);
    assert_eq!(p.run(&data), 64);
    let p = ok(&[Insn::stmt(LD | W | IMM, 42), Insn::stmt(RET | A, 0)]);
    assert_eq!(p.run(&data), 42);
    // A short input reads as zeros past its end.
    let p = ok(&[Insn::stmt(LD | W | ABS, 8), Insn::stmt(RET | A, 0)]);
    assert_eq!(p.run(&[1]), 0);
}

#[test]
fn scratch_memory_holds_words_and_starts_zeroed() {
    let p = ok(&[
        Insn::stmt(LD | W | MEM, 15),
        Insn::stmt(ST, 3),
        Insn::stmt(LD | W | IMM, 9),
        Insn::stmt(ST, 4),
        Insn::stmt(LDX | W | MEM, 4),
        Insn::stmt(LD | W | MEM, 3),
        Insn::stmt(ALU | ADD | X, 0),
        Insn::stmt(STX, 15),
        Insn::stmt(RET | A, 0),
    ]);
    assert_eq!(p.run(&[]), 9);
    assert_eq!(err(&[Insn::stmt(ST, 16), ret(0)]), Error::BadMemory { pc: 0 });
    assert_eq!(err(&[Insn::stmt(LD | W | MEM, 16), ret(0)]), Error::BadMemory { pc: 0 });
    assert_eq!(err(&[Insn::stmt(LDX | W | MEM, 99), ret(0)]), Error::BadMemory { pc: 0 });
}

fn alu(op: u16, a: u32, k: u32) -> u32 {
    ok(&[Insn::stmt(LD | W | IMM, a), Insn::stmt(ALU | op | K, k), Insn::stmt(RET | A, 0)]).run(&[])
}

fn alu_x(op: u16, a: u32, x: u32) -> u32 {
    ok(&[
        Insn::stmt(LD | W | IMM, a),
        Insn::stmt(LDX | W | IMM, x),
        Insn::stmt(ALU | op | X, 0),
        Insn::stmt(RET | A, 0),
    ])
    .run(&[])
}

#[test]
fn arithmetic_is_unsigned_32_bit() {
    assert_eq!(alu(ADD, u32::MAX, 2), 1);
    assert_eq!(alu(SUB, 1, 2), u32::MAX);
    assert_eq!(alu(MUL, 0x1_0000, 0x1_0000), 0);
    assert_eq!(alu(DIV, 7, 2), 3);
    assert_eq!(alu(MOD, 7, 4), 3);
    assert_eq!(alu(OR, 0xf0, 0x0f), 0xff);
    assert_eq!(alu(AND, 0xf0, 0x3c), 0x30);
    assert_eq!(alu(XOR, 0xff, 0x0f), 0xf0);
    assert_eq!(alu(LSH, 1, 31), 0x8000_0000);
    assert_eq!(alu(RSH, 0x8000_0000, 31), 1);
    let neg = ok(&[Insn::stmt(LD | W | IMM, 1), Insn::stmt(ALU | NEG, 0), Insn::stmt(RET | A, 0)]);
    assert_eq!(neg.run(&[]), u32::MAX);
    assert_eq!(alu_x(SUB, 10, 3), 7);
    assert_eq!(alu_x(LSH, 1, 40), 0);
    assert_eq!(alu_x(RSH, 1, 32), 0);
}

#[test]
fn division_by_a_zero_x_returns_zero() {
    assert_eq!(alu_x(DIV, 7, 0), 0);
    assert_eq!(alu_x(MOD, 7, 0), 0);
    // Even when the program would return something else afterwards.
    let p = ok(&[Insn::stmt(ALU | DIV | X, 0), ret(5)]);
    assert_eq!(p.run(&[]), 0);
}

#[test]
fn constant_division_by_zero_and_wide_shifts_are_refused() {
    assert_eq!(err(&[Insn::stmt(ALU | DIV | K, 0), ret(0)]), Error::BadConstant { pc: 0 });
    assert_eq!(err(&[Insn::stmt(ALU | MOD | K, 0), ret(0)]), Error::BadConstant { pc: 0 });
    assert_eq!(err(&[Insn::stmt(ALU | LSH | K, 32), ret(0)]), Error::BadConstant { pc: 0 });
    assert_eq!(err(&[Insn::stmt(ALU | RSH | K, 99), ret(0)]), Error::BadConstant { pc: 0 });
}

#[test]
fn conditional_jumps_compare_with_k_or_x() {
    let cmp = |op: u16, a: u32, k: u32| {
        ok(&[Insn::stmt(LD | W | IMM, a), Insn::jump(JMP | op | K, k, 1, 0), ret(0), ret(1)]).run(&[])
    };
    assert_eq!(cmp(JEQ, 5, 5), 1);
    assert_eq!(cmp(JEQ, 5, 6), 0);
    assert_eq!(cmp(JGT, 6, 5), 1);
    assert_eq!(cmp(JGT, 5, 5), 0);
    assert_eq!(cmp(JGE, 5, 5), 1);
    assert_eq!(cmp(JGE, 4, 5), 0);
    assert_eq!(cmp(JSET, 0b1010, 0b0010), 1);
    assert_eq!(cmp(JSET, 0b1010, 0b0101), 0);
    // Unsigned: 0xffffffff is greater than 1.
    assert_eq!(cmp(JGT, u32::MAX, 1), 1);
    let with_x = ok(&[
        Insn::stmt(LD | W | IMM, 3),
        Insn::stmt(LDX | W | IMM, 3),
        Insn::jump(JMP | JEQ | X, 0, 0, 1),
        ret(10),
        ret(20),
    ]);
    assert_eq!(with_x.run(&[]), 10);
}

#[test]
fn jumps_must_land_inside_the_program() {
    // ja over one instruction to the last: fine; one further: refused.
    ok(&[Insn::stmt(JMP | JA, 1), ret(1), ret(2)]);
    assert_eq!(err(&[Insn::stmt(JMP | JA, 2), ret(1), ret(2)]), Error::BadJump { pc: 0 });
    assert_eq!(err(&[Insn::stmt(JMP | JA, u32::MAX), ret(1)]), Error::BadJump { pc: 0 });
    assert_eq!(err(&[Insn::jump(JMP | JEQ | K, 0, 2, 0), ret(1), ret(2)]), Error::BadJump { pc: 0 });
    assert_eq!(err(&[Insn::jump(JMP | JEQ | K, 0, 0, 2), ret(1), ret(2)]), Error::BadJump { pc: 0 });
    let p = ok(&[Insn::stmt(JMP | JA, 1), ret(1), ret(2)]);
    assert_eq!(p.run(&[]), 2);
}

#[test]
fn the_last_instruction_must_return() {
    assert_eq!(err(&[Insn::stmt(LD | W | IMM, 1)]), Error::NoReturn);
    assert_eq!(err(&[ret(0), Insn::stmt(LD | W | IMM, 1)]), Error::NoReturn);
    assert_eq!(err(&[ret(0), Insn::stmt(MISC | TAX, 0)]), Error::NoReturn);
}

#[test]
fn unknown_opcodes_are_refused() {
    for code in [
        0x0400 | RET, // a bit above the classic fields
        ALU | 0xb0,
        ALU | NEG | X,
        JMP | 0x50,
        JMP | JA | X,
        RET | 0x18,
        MISC | 0x40,
        ST | 0x08,
        STX | 0x60,
        LDX | W | ABS,
        LD | H | LEN,
    ] {
        assert_eq!(err(&[Insn::stmt(code, 0), ret(0)]), Error::BadOpcode { pc: 0 }, "{code:#x}");
    }
}

#[test]
fn registers_move_and_return() {
    let p = ok(&[
        Insn::stmt(LD | W | IMM, 77),
        Insn::stmt(MISC | TAX, 0),
        Insn::stmt(LD | W | IMM, 1),
        Insn::stmt(RET | X, 0),
    ]);
    assert_eq!(p.run(&[]), 77);
    let p = ok(&[Insn::stmt(LDX | W | IMM, 78), Insn::stmt(MISC | TXA, 0), Insn::stmt(RET | A, 0)]);
    assert_eq!(p.run(&[]), 78);
}

#[test]
fn instructions_decode_from_their_bytes() {
    let bytes = [0x20, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x15, 0x00, 0x01, 0x02, 0x3e, 0x00, 0x00, 0xc0];
    let insns = Insn::parse(&bytes);
    assert_eq!(insns, vec![Insn::stmt(LD | W | ABS, 4), Insn::jump(JMP | JEQ | K, 0xc000_003e, 1, 2)]);
}
