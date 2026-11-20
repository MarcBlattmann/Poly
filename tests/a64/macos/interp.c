#include "rt.h"
/* A tiny bytecode interpreter: a switch (jump table, `br`) and function pointers (`blr`). */
enum { PUSH, ADD, SUB, MUL, DUP, SWAP, JNZ, DEC, PRINT, HALT, CALL, DROP };
static i64 sq(i64 x) { return x * x; }
static i64 neg(i64 x) { return -x; }
static i64 (*fns[])(i64) = {sq, neg};
static i64 exec(const i64 *code) {
    i64 st[64]; int sp = 0, pc = 0; i64 acc = 0;
    for (;;) {
        switch (code[pc++]) {
        case PUSH: st[sp++] = code[pc++]; break;
        case ADD: sp--; st[sp-1] += st[sp]; break;
        case SUB: sp--; st[sp-1] -= st[sp]; break;
        case MUL: sp--; st[sp-1] *= st[sp]; break;
        case DUP: st[sp] = st[sp-1]; sp++; break;
        case SWAP: { i64 t = st[sp-1]; st[sp-1] = st[sp-2]; st[sp-2] = t; } break;
        case JNZ: { i64 tgt = code[pc++]; if (st[sp-1]) pc = (int)tgt; } break;
        case DEC: st[sp-1]--; break;
        case PRINT: acc = acc * 31 + st[sp-1]; break;
        case CALL: st[sp-1] = fns[code[pc++]](st[sp-1]); break;
        case DROP: sp--; break;
        case HALT: return acc;
        }
    }
}
static int fib(int n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int run(void) {
    const i64 prog[] = {PUSH, 50000, /*2*/ DUP, CALL, 0, CALL, 1, PUSH, 3, MUL, PRINT, DROP, DEC, JNZ, 2, HALT};
    puts_("bytecode="); puti(exec(prog)); nl();
    puts_("fib(27)="); puti(fib(27)); nl();
    return 0;
}
