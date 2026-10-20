//! Processes and their personality tag.
//!
//! The personality is decided once, at exec, from the image's binary format,
//! and every system call the process makes is routed by it.

use poly_abi::Personality;
use poly_binfmt as binfmt;

#[derive(Debug)]
pub struct Process {
    pub pid: u32,
    pub personality: Personality,
    pub format: binfmt::Format,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExecError {
    Format(binfmt::Error),
    /// `#!` scripts are resolved by the exec path, not here.
    Script,
}

impl Process {
    /// A process record for a personality and format decided elsewhere.
    pub fn new(pid: u32, personality: Personality, format: binfmt::Format) -> Process {
        Process { pid, personality, format }
    }

    /// Create the process record for an image. Loading and mapping the
    /// image is the personality's job and comes later.
    pub fn from_image(pid: u32, image: &[u8]) -> Result<Process, ExecError> {
        let format = binfmt::detect(image).map_err(ExecError::Format)?;
        let personality = format.personality().ok_or(ExecError::Script)?;
        Ok(Process { pid, personality, format })
    }
}
