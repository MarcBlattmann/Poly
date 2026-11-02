//! `/bin/wl-client` as a native program (ADR 0125, ADR 0133). Its code is
//! the `poly-wl-demo` crate.

#![no_std]
#![no_main]

poly_rt::entry!(poly_wl_demo::client::main);
