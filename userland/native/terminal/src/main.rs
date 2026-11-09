//! `/bin/poly-terminal`: Poly's own terminal emulator, as a native program (ADR 0134). Its code is
//! the `poly-terminal` crate.

#![no_std]
#![no_main]

poly_rt::entry!(poly_terminal::main);
