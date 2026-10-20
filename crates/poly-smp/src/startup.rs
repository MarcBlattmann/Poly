//! Starting an application processor: INIT, then two start-up IPIs.
//!
//! The "universal start-up algorithm" (MultiProcessor Specification 1.4,
//! appendix B.4) and the SDM's typical BSP sequence (Vol. 3A §9.4.4.1):
//!
//! 1. send an INIT IPI to the AP and wait 10 ms;
//! 2. send a start-up IPI (SIPI) with the vector of the start-up page and
//!    wait 200 µs;
//! 3. send a second SIPI and wait 200 µs: a processor that already started
//!    ignores it (SDM Vol. 3A §11.6.1: "the SIPI is not latched"), but some
//!    need it. This module skips it when the AP has checked in already;
//! 4. wait for the AP to report that it runs, with a timeout.
//!
//! The kernel implements [`Startup`] with its local APIC (the ICR) and a
//! clock; the order, the delays and the timeout are tested here.

/// What the sequence needs from the machine.
pub trait Startup {
    type Error;
    /// Send an INIT IPI to the processor with APIC ID `apic_id`.
    fn send_init(&mut self, apic_id: u32) -> Result<(), Self::Error>;
    /// Send a start-up IPI with `vector` (the start-up page number).
    fn send_startup(&mut self, apic_id: u32, vector: u8) -> Result<(), Self::Error>;
    /// Wait at least `us` microseconds.
    fn delay_us(&mut self, us: u64);
    /// Microseconds on a monotonic clock.
    fn now_us(&mut self) -> u64;
    /// Whether the AP has reported that it runs kernel code.
    fn started(&mut self) -> bool;
}

/// The SDM's delays: 10 ms after INIT, 200 µs after each SIPI.
pub const INIT_DELAY_US: u64 = 10_000;
pub const SIPI_DELAY_US: u64 = 200;

/// Why an AP did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError<E> {
    /// An IPI could not be sent (the APIC did not accept it).
    Ipi(E),
    /// The AP did not report in time.
    Timeout,
}

/// How the start went: how many start-up IPIs it took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Started {
    pub sipis: u8,
}

/// Start the AP with APIC ID `apic_id` at start-up page `vector`, waiting
/// up to `timeout_us` after the last SIPI for it to report.
pub fn start_ap<S: Startup>(s: &mut S, apic_id: u32, vector: u8, timeout_us: u64) -> Result<Started, StartError<S::Error>> {
    s.send_init(apic_id).map_err(StartError::Ipi)?;
    s.delay_us(INIT_DELAY_US);
    let mut sipis = 0;
    for _ in 0..2 {
        if s.started() {
            break;
        }
        s.send_startup(apic_id, vector).map_err(StartError::Ipi)?;
        sipis += 1;
        s.delay_us(SIPI_DELAY_US);
    }
    let start = s.now_us();
    while !s.started() {
        if s.now_us().saturating_sub(start) > timeout_us {
            return Err(StartError::Timeout);
        }
        core::hint::spin_loop();
    }
    Ok(Started { sipis })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[derive(Debug, PartialEq, Eq)]
    enum Step {
        Init(u32),
        Sipi(u32, u8),
        Delay(u64),
    }

    /// A simulated AP that starts after the `starts_after`-th SIPI (never
    /// with `None`), on a clock that only the delays and polls move.
    struct Sim {
        log: Vec<Step>,
        sipis: u8,
        starts_after: Option<u8>,
        now: u64,
        fail_init: bool,
    }

    impl Sim {
        fn new(starts_after: Option<u8>) -> Sim {
            Sim { log: Vec::new(), sipis: 0, starts_after, now: 0, fail_init: false }
        }
    }

    impl Startup for Sim {
        type Error = &'static str;
        fn send_init(&mut self, apic_id: u32) -> Result<(), &'static str> {
            if self.fail_init {
                return Err("icr busy");
            }
            self.log.push(Step::Init(apic_id));
            Ok(())
        }
        fn send_startup(&mut self, apic_id: u32, vector: u8) -> Result<(), &'static str> {
            self.sipis += 1;
            self.log.push(Step::Sipi(apic_id, vector));
            Ok(())
        }
        fn delay_us(&mut self, us: u64) {
            self.now += us;
            self.log.push(Step::Delay(us));
        }
        fn now_us(&mut self) -> u64 {
            self.now += 10;
            self.now
        }
        fn started(&mut self) -> bool {
            self.starts_after.is_some_and(|n| self.sipis >= n)
        }
    }

    #[test]
    fn init_then_two_sipis_with_the_documented_delays() {
        let mut sim = Sim::new(Some(2));
        assert_eq!(start_ap(&mut sim, 3, 0x08, 1_000_000), Ok(Started { sipis: 2 }));
        assert_eq!(
            sim.log,
            [Step::Init(3), Step::Delay(10_000), Step::Sipi(3, 8), Step::Delay(200), Step::Sipi(3, 8), Step::Delay(200)]
        );
    }

    #[test]
    fn an_ap_that_started_on_the_first_sipi_gets_no_second() {
        let mut sim = Sim::new(Some(1));
        assert_eq!(start_ap(&mut sim, 1, 0x9c, 1_000_000), Ok(Started { sipis: 1 }));
        assert_eq!(sim.log.iter().filter(|s| matches!(s, Step::Sipi(..))).count(), 1);
    }

    #[test]
    fn a_silent_ap_times_out_and_ipi_errors_are_reported() {
        let mut sim = Sim::new(None);
        assert_eq!(start_ap(&mut sim, 1, 8, 5_000), Err(StartError::Timeout));
        assert!(sim.now >= 10_000 + 400 + 5_000, "waited the whole timeout");
        let mut broken = Sim::new(Some(1));
        broken.fail_init = true;
        assert_eq!(start_ap(&mut broken, 1, 8, 5_000), Err(StartError::Ipi("icr busy")));
        assert!(broken.log.is_empty());
    }
}
