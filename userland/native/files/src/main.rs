//! `/bin/poly-files`: Poly's own file explorer, Files, as a native program (ADR 0134). Its code is
//! the `poly-files-app` crate.

#![no_std]
#![no_main]

poly_rt::entry!(poly_files_app::main);
