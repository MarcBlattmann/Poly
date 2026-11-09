//! Power State Coordination Interface (Arm DEN0022): the one call the stub
//! makes when it has nothing left to do and no semihosting to say so.

/// `SYSTEM_OFF` in the SMC32/HVC32 calling convention (DEN0022, "Function
/// IDs": 0x8400_0008).
pub const SYSTEM_OFF: u64 = 0x8400_0008;

/// How the firmware is called, from the devicetree's PSCI `method`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conduit {
    Hvc,
    Smc,
}

impl Conduit {
    /// `"hvc"` or `"smc"`; anything else is not a conduit.
    pub fn parse(method: &str) -> Option<Conduit> {
        match method {
            "hvc" => Some(Conduit::Hvc),
            "smc" => Some(Conduit::Smc),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_conduit() {
        assert_eq!(Conduit::parse("hvc"), Some(Conduit::Hvc));
        assert_eq!(Conduit::parse("smc"), Some(Conduit::Smc));
        assert_eq!(Conduit::parse("HVC"), None);
        assert_eq!(Conduit::parse(""), None);
    }
}
