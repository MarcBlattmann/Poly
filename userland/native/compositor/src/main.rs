//! `/bin/poly-compositor`: Poly's compositor and desktop as a native
//! program (ADR 0124, ADR 0125). Its code is the `poly-compositor` crate.

#![no_std]
#![no_main]

poly_rt::entry!(poly_compositor::main);
