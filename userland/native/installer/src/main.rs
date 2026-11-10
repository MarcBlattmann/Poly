//! `/bin/poly-installer`: Poly's installer as a native program (ADR 0173). Its code is the
//! `poly-installer-app` crate.

#![no_std]
#![no_main]

poly_rt::entry!(poly_installer_app::main);
