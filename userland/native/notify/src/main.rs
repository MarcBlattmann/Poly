//! `/bin/notify-send` as a native program (ADR 0125). Its code is the
//! `poly-notify` crate.

#![no_std]
#![no_main]

poly_rt::entry!(poly_notify::main);
