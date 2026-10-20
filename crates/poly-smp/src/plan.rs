//! Which processors to start.
//!
//! The ACPI MADT lists every processor the firmware made available (Local
//! APIC and Local x2APIC structures with the "enabled" flag, ACPI 6.5
//! §5.2.12.2 and §5.2.12.12). The kernel numbers the CPUs it runs on
//! densely: CPU 0 is the boot processor (BSP), whatever its APIC ID, and
//! the application processors follow in MADT order. Command-line options
//! limit how many:
//!
//! - `poly.smp=off`: only the boot processor (`poly.smp=on` is the default).
//! - `poly.cpus=<n>`: at most `n` CPUs, the boot processor included.

use alloc::vec::Vec;

/// One processor as the MADT describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Processor {
    /// The ACPI processor UID.
    pub uid: u32,
    /// Its local APIC ID.
    pub apic_id: u32,
}

/// What the command line asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `poly.smp=off`: the boot processor only.
    pub off: bool,
    /// `poly.cpus=<n>`: at most this many CPUs.
    pub max_cpus: Option<usize>,
}

/// A command-line option with a value it does not understand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionError {
    /// `poly.smp=` takes `on` or `off`.
    Smp,
    /// `poly.cpus=` takes a number of at least 1.
    Cpus,
}

impl OptionError {
    pub const fn describe(self) -> &'static str {
        match self {
            OptionError::Smp => "poly.smp= takes on or off",
            OptionError::Cpus => "poly.cpus= takes a number from 1 up",
        }
    }
}

/// Parse the values of `poly.smp=` and `poly.cpus=` (each `None` when the
/// option is absent).
pub fn parse_options(smp: Option<&str>, cpus: Option<&str>) -> Result<Options, OptionError> {
    let off = match smp {
        None | Some("on") => false,
        Some("off") => true,
        Some(_) => return Err(OptionError::Smp),
    };
    let max_cpus = match cpus {
        None => None,
        Some(n) => match n.parse::<usize>() {
            Ok(n) if n >= 1 => Some(n),
            _ => return Err(OptionError::Cpus),
        },
    };
    Ok(Options { off, max_cpus })
}

/// The CPUs to run on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Index 0 is the boot processor; the rest are the APs to start, in
    /// MADT order. A CPU's index here is its CPU number.
    pub cpus: Vec<Processor>,
    /// Processors left out by the options or the kernel's limit.
    pub left_out: usize,
    /// Processors whose APIC ID the APIC cannot address in xAPIC mode
    /// (above 254; 255 is the broadcast ID).
    pub unreachable: usize,
}

impl Plan {
    /// The application processors to start (every CPU but the first).
    pub fn aps(&self) -> &[Processor] {
        self.cpus.get(1..).unwrap_or(&[])
    }
}

/// Highest APIC ID an IPI can name in xAPIC mode: the destination field is
/// 8 bits and 0xff broadcasts (SDM Vol. 3A §11.6.1).
pub const XAPIC_MAX_ID: u32 = 0xfe;

/// Which processors to run on: the boot processor (APIC ID
/// `bsp_apic_id`) first, then the other enabled processors of
/// `processors` in order, without duplicates, up to `limit` CPUs in all
/// (the kernel's table size) and what `options` allow. In xAPIC mode
/// (`x2apic` false) processors with an APIC ID above
/// [`XAPIC_MAX_ID`] cannot be started and are counted as unreachable.
pub fn plan(processors: &[Processor], bsp_apic_id: u32, options: Options, limit: usize, x2apic: bool) -> Plan {
    let mut max = limit.max(1);
    if let Some(n) = options.max_cpus {
        max = max.min(n);
    }
    if options.off {
        max = 1;
    }
    let bsp = processors.iter().copied().find(|p| p.apic_id == bsp_apic_id);
    let mut cpus = Vec::with_capacity(processors.len().min(max).max(1));
    cpus.push(bsp.unwrap_or(Processor { uid: 0, apic_id: bsp_apic_id }));
    let (mut left_out, mut unreachable) = (0, 0);
    for (i, p) in processors.iter().enumerate() {
        let duplicate = processors[..i].iter().any(|q| q.apic_id == p.apic_id);
        if p.apic_id == bsp_apic_id || duplicate {
            continue;
        }
        if !x2apic && p.apic_id > XAPIC_MAX_ID {
            unreachable += 1;
        } else if cpus.len() < max {
            cpus.push(*p);
        } else {
            left_out += 1;
        }
    }
    Plan { cpus, left_out, unreachable }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn p(uid: u32, apic_id: u32) -> Processor {
        Processor { uid, apic_id }
    }

    const ALL: Options = Options { off: false, max_cpus: None };

    #[test]
    fn options_parse() {
        assert_eq!(parse_options(None, None), Ok(Options::default()));
        assert_eq!(parse_options(Some("off"), None), Ok(Options { off: true, max_cpus: None }));
        assert_eq!(parse_options(Some("on"), Some("3")), Ok(Options { off: false, max_cpus: Some(3) }));
        assert_eq!(parse_options(Some("yes"), None), Err(OptionError::Smp));
        assert_eq!(parse_options(None, Some("0")), Err(OptionError::Cpus));
        assert_eq!(parse_options(None, Some("two")), Err(OptionError::Cpus));
        assert!(!OptionError::Cpus.describe().is_empty() && !OptionError::Smp.describe().is_empty());
    }

    #[test]
    fn the_boot_processor_comes_first_whatever_its_id() {
        let madt = [p(0, 0), p(1, 2), p(2, 4), p(3, 6)];
        let plan = plan(&madt, 4, ALL, 64, false);
        assert_eq!(plan.cpus, [p(2, 4), p(0, 0), p(1, 2), p(3, 6)]);
        assert_eq!(plan.aps(), [p(0, 0), p(1, 2), p(3, 6)]);
        assert_eq!((plan.left_out, plan.unreachable), (0, 0));
    }

    #[test]
    fn options_and_the_limit_cap_the_count() {
        let madt = [p(0, 0), p(1, 1), p(2, 2), p(3, 3)];
        let off = plan(&madt, 0, Options { off: true, max_cpus: Some(4) }, 64, false);
        assert_eq!((off.cpus.len(), off.left_out), (1, 3));
        assert!(off.aps().is_empty());
        let two = plan(&madt, 0, Options { off: false, max_cpus: Some(2) }, 64, false);
        assert_eq!(two.cpus, [p(0, 0), p(1, 1)]);
        assert_eq!(two.left_out, 2);
        let limited = plan(&madt, 0, ALL, 3, false);
        assert_eq!((limited.cpus.len(), limited.left_out), (3, 1));
        assert_eq!(plan(&madt, 0, ALL, 0, false).cpus.len(), 1, "the boot processor always runs");
    }

    #[test]
    fn duplicates_and_unaddressable_ids_are_skipped() {
        let madt = [p(0, 0), p(1, 1), p(9, 1), p(2, 300), p(3, 255), p(4, 254)];
        let x = plan(&madt, 0, ALL, 64, false);
        assert_eq!(x.cpus, [p(0, 0), p(1, 1), p(4, 254)]);
        assert_eq!(x.unreachable, 2);
        let x2 = plan(&madt, 0, ALL, 64, true);
        assert_eq!(x2.cpus.len(), 5, "x2APIC mode reaches every ID");
        assert_eq!(x2.unreachable, 0);
    }

    #[test]
    fn a_boot_processor_missing_from_the_madt_still_runs() {
        let plan = plan(&[p(5, 1)], 0, ALL, 64, false);
        assert_eq!(plan.cpus, vec![p(0, 0), p(5, 1)]);
        let none = super::plan(&[], 7, ALL, 64, false);
        assert_eq!(none.cpus, vec![p(0, 7)]);
    }
}
