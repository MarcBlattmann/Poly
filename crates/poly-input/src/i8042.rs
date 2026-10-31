//! The i8042 (8042-compatible) PS/2 controller and the keyboard behind it:
//! register bits, commands and the bring-up sequence, behind an access
//! trait ([`Io`]) so it runs against a simulated controller in the tests.
//!
//! Sources: IBM, "Personal System/2 Hardware Interface Technical Reference"
//! (keyboard/auxiliary device controller and keyboard chapters); the
//! controller's two ports, status bits, command bytes and configuration
//! ("command") byte as documented there and on the OSDev wiki pages "8042
//! PS/2 Controller" and "PS/2 Keyboard".
//!
//! The controller has a data port ([`DATA_PORT`], 0x60) and a status /
//! command port ([`STATUS_PORT`], 0x64). A byte from a device (or a reply
//! to a controller command) can be read from the data port when status bit
//! 0 is set; a command or data byte can be written when status bit 1 is
//! clear. Every wait here is bounded by time ([`Io::now_ns`]) and by a
//! number of polls, so a missing or wedged controller costs at most a
//! fraction of a second and never hangs.

/// Data port: device bytes and replies in, device bytes and command
/// parameters out.
pub const DATA_PORT: u16 = 0x60;
/// Status register (read) and command register (write).
pub const STATUS_PORT: u16 = 0x64;

// Status register bits.
/// A byte is waiting in the output buffer (read it from the data port).
pub const STATUS_OUTPUT_FULL: u8 = 1 << 0;
/// The controller has not taken the last byte written yet.
pub const STATUS_INPUT_FULL: u8 = 1 << 1;
/// The waiting byte came from the second (mouse) port.
pub const STATUS_AUX_DATA: u8 = 1 << 5;
pub const STATUS_TIMEOUT: u8 = 1 << 6;
pub const STATUS_PARITY: u8 = 1 << 7;

// Controller commands (written to the command port).
pub const CMD_READ_CONFIG: u8 = 0x20;
pub const CMD_WRITE_CONFIG: u8 = 0x60;
pub const CMD_DISABLE_AUX: u8 = 0xa7;
pub const CMD_ENABLE_AUX: u8 = 0xa8;
pub const CMD_TEST_AUX_PORT: u8 = 0xa9;
pub const CMD_SELF_TEST: u8 = 0xaa;
pub const CMD_TEST_KEYBOARD_PORT: u8 = 0xab;
pub const CMD_DISABLE_KEYBOARD: u8 = 0xad;
pub const CMD_ENABLE_KEYBOARD: u8 = 0xae;
/// The next data byte appears in the output buffer as if the keyboard had
/// sent it (raising IRQ 1 if enabled).
pub const CMD_WRITE_KEYBOARD_OUTPUT: u8 = 0xd2;
/// The next data byte appears in the output buffer as if the second-port
/// device (the mouse) had sent it (raising IRQ 12 if enabled).
pub const CMD_WRITE_AUX_OUTPUT: u8 = 0xd3;
/// The next data byte goes to the second-port device (the mouse).
pub const CMD_WRITE_AUX: u8 = 0xd4;

/// Self-test passed.
pub const SELF_TEST_OK: u8 = 0x55;
/// Port test passed (1-4 name a stuck clock or data line).
pub const PORT_TEST_OK: u8 = 0x00;

// Configuration byte bits.
pub const CONFIG_KEYBOARD_IRQ: u8 = 1 << 0;
pub const CONFIG_AUX_IRQ: u8 = 1 << 1;
/// Set by firmware once the power-on self-test passed.
pub const CONFIG_SYSTEM_FLAG: u8 = 1 << 2;
pub const CONFIG_KEYBOARD_CLOCK_OFF: u8 = 1 << 4;
pub const CONFIG_AUX_CLOCK_OFF: u8 = 1 << 5;
/// Translate the keyboard's scancode set 2 into set 1.
pub const CONFIG_TRANSLATE: u8 = 1 << 6;

// Keyboard commands (written to the data port).
pub const KBD_SET_LEDS: u8 = 0xed;
pub const KBD_ECHO: u8 = 0xee;
pub const KBD_SCANCODE_SET: u8 = 0xf0;
pub const KBD_ENABLE_SCANNING: u8 = 0xf4;
pub const KBD_DISABLE_SCANNING: u8 = 0xf5;
pub const KBD_RESET: u8 = 0xff;

// Keyboard replies.
pub const ACK: u8 = 0xfa;
pub const RESEND: u8 = 0xfe;
/// Basic assurance test (the keyboard's self-test after reset) passed.
pub const BAT_OK: u8 = 0xaa;
pub const BAT_FAILED: [u8; 2] = [0xfc, 0xfd];

/// How long the controller may take to accept a byte or answer a command.
pub const CONTROLLER_TIMEOUT_NS: u64 = 100_000_000;
/// How long the keyboard may take to acknowledge a command.
pub const KEYBOARD_TIMEOUT_NS: u64 = 250_000_000;
/// How long the keyboard's self-test after a reset may take.
pub const RESET_TIMEOUT_NS: u64 = 1_000_000_000;
/// Polls of the status register any single wait gives up after, even if
/// the clock does not move.
pub const MAX_POLLS: u32 = 1 << 22;
/// Times a command is sent again when the keyboard answers RESEND.
pub const RETRIES: u32 = 3;

/// Access to the controller's two ports and a clock.
pub trait Io {
    fn read_status(&self) -> u8;
    fn read_data(&self) -> u8;
    fn write_command(&self, command: u8);
    fn write_data(&self, byte: u8);
    /// A monotonic clock in nanoseconds, for timeouts.
    fn now_ns(&self) -> u64;
}

/// Why the controller or keyboard could not be brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The status register reads 0xFF: nothing decodes the ports.
    NoController,
    /// The controller did not accept a byte or answer in time; names the
    /// step.
    Timeout(&'static str),
    /// The controller self-test answered this instead of 0x55.
    SelfTest(u8),
    /// The keyboard port test answered this instead of 0x00 (1: clock
    /// stuck low, 2: clock stuck high, 3: data stuck low, 4: data stuck
    /// high).
    PortTest(u8),
    /// Nothing acknowledged the reset: no keyboard is plugged in.
    NoKeyboard,
    /// The keyboard's self-test failed (0xFC or 0xFD).
    KeyboardSelfTest(u8),
    /// The keyboard did not acknowledge this command.
    Refused(u8),
}

/// What [`init`] found and left behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    /// The configuration byte as firmware left it, read after disabling
    /// both ports (which sets their clock-off bits).
    pub firmware_config: u8,
    /// The configuration byte [`enable_interrupts`] writes: keyboard
    /// interrupt and translation on, the second port's interrupt off and
    /// its clock stopped.
    pub config: u8,
    /// The keyboard acknowledged switching to scancode set 2 (which the
    /// controller translates to set 1). A keyboard that refuses stays in
    /// its default set, which is set 2 after a reset anyway.
    pub set2_acknowledged: bool,
    /// Bytes thrown away while flushing the output buffer.
    pub flushed: u32,
}

/// Wait for `status & mask == want`.
fn wait_status(io: &impl Io, mask: u8, want: u8, timeout_ns: u64) -> bool {
    let start = io.now_ns();
    for _ in 0..MAX_POLLS {
        if io.read_status() & mask == want {
            return true;
        }
        if io.now_ns().wrapping_sub(start) > timeout_ns {
            break;
        }
        core::hint::spin_loop();
    }
    io.read_status() & mask == want
}

/// Wait until the controller can take a byte.
fn wait_input_empty(io: &impl Io, step: &'static str) -> Result<(), Error> {
    if wait_status(io, STATUS_INPUT_FULL, 0, CONTROLLER_TIMEOUT_NS) { Ok(()) } else { Err(Error::Timeout(step)) }
}

/// The next byte from the output buffer, waiting up to `timeout_ns`.
pub fn read_byte(io: &impl Io, timeout_ns: u64) -> Option<u8> {
    wait_status(io, STATUS_OUTPUT_FULL, STATUS_OUTPUT_FULL, timeout_ns).then(|| io.read_data())
}

/// Write a controller command (when the controller can take it).
pub fn command(io: &impl Io, cmd: u8) -> Result<(), Error> {
    wait_input_empty(io, "controller command")?;
    io.write_command(cmd);
    Ok(())
}

/// Write a controller command and read its one-byte answer.
fn command_reply(io: &impl Io, cmd: u8, step: &'static str) -> Result<u8, Error> {
    command(io, cmd)?;
    read_byte(io, CONTROLLER_TIMEOUT_NS).ok_or(Error::Timeout(step))
}

/// Write a byte to the data port: a parameter of the last controller
/// command, or a byte for the keyboard.
pub fn write_data(io: &impl Io, byte: u8) -> Result<(), Error> {
    wait_input_empty(io, "data byte")?;
    io.write_data(byte);
    Ok(())
}

pub fn read_config(io: &impl Io) -> Result<u8, Error> {
    command_reply(io, CMD_READ_CONFIG, "reading the configuration byte")
}

pub fn write_config(io: &impl Io, config: u8) -> Result<(), Error> {
    command(io, CMD_WRITE_CONFIG)?;
    write_data(io, config)
}

/// Make `byte` appear in the output buffer as if the keyboard had sent it
/// (raising IRQ 1 if it is enabled). For testing the interrupt path.
pub fn write_keyboard_output(io: &impl Io, byte: u8) -> Result<(), Error> {
    command(io, CMD_WRITE_KEYBOARD_OUTPUT)?;
    write_data(io, byte)
}

/// Read and discard whatever is waiting, up to 64 bytes; returns how many.
pub fn flush(io: &impl Io) -> u32 {
    let mut n = 0;
    while n < 64 && io.read_status() & STATUS_OUTPUT_FULL != 0 {
        io.read_data();
        n += 1;
    }
    n
}

/// Send a byte to the keyboard and wait for its acknowledgement, sending
/// it again when the keyboard asks to (RESEND). Other bytes that arrive
/// meanwhile (keys pressed during bring-up) are dropped. For use while the
/// keyboard interrupt is off; with it on, the interrupt handler gets the
/// reply.
pub fn keyboard_command(io: &impl Io, byte: u8) -> Result<(), Error> {
    for _ in 0..=RETRIES {
        write_data(io, byte)?;
        let start = io.now_ns();
        loop {
            let left = KEYBOARD_TIMEOUT_NS.saturating_sub(io.now_ns().wrapping_sub(start));
            match read_byte(io, left) {
                Some(ACK) => return Ok(()),
                Some(RESEND) => break,
                Some(_) if left > 0 => continue,
                _ => return Err(Error::Refused(byte)),
            }
        }
    }
    Err(Error::Refused(byte))
}

/// Bring up the controller and the keyboard on its first port, polled,
/// with both ports' interrupts off:
///
/// 1. Check something answers at the status port (0xFF: no controller).
/// 2. Disable both ports, so no device writes into the output buffer.
/// 3. Flush the output buffer.
/// 4. Read the configuration byte; write it back with both interrupts and
///    translation off.
/// 5. Controller self-test (0xAA, expect 0x55), and write the
///    configuration again: the self-test may reset the controller.
/// 6. Test the keyboard port (0xAB, expect 0x00) and enable it.
/// 7. Reset the keyboard (0xFF: ACK, then 0xAA from its self-test).
/// 8. Select scancode set 2 (0xF0 0x02), which the controller translates
///    to set 1, and enable scanning (0xF4).
///
/// The second (mouse) port stays disabled. Then [`enable_interrupts`]
/// turns translation and the keyboard interrupt on.
pub fn init(io: &impl Io) -> Result<Setup, Error> {
    if io.read_status() == 0xff {
        return Err(Error::NoController);
    }
    command(io, CMD_DISABLE_KEYBOARD)?;
    command(io, CMD_DISABLE_AUX)?;
    let flushed = flush(io);

    let firmware_config = read_config(io)?;
    let quiet = firmware_config & !(CONFIG_KEYBOARD_IRQ | CONFIG_AUX_IRQ | CONFIG_TRANSLATE);
    write_config(io, quiet)?;

    match command_reply(io, CMD_SELF_TEST, "controller self-test")? {
        SELF_TEST_OK => {}
        other => return Err(Error::SelfTest(other)),
    }
    write_config(io, quiet)?;

    match command_reply(io, CMD_TEST_KEYBOARD_PORT, "keyboard port test")? {
        PORT_TEST_OK => {}
        other => return Err(Error::PortTest(other)),
    }
    command(io, CMD_ENABLE_KEYBOARD)?;

    reset_keyboard(io)?;
    let set2_acknowledged = keyboard_command(io, KBD_SCANCODE_SET).and_then(|()| keyboard_command(io, 2)).is_ok();
    keyboard_command(io, KBD_ENABLE_SCANNING)?;

    let config = (quiet | CONFIG_KEYBOARD_IRQ | CONFIG_TRANSLATE | CONFIG_AUX_CLOCK_OFF) & !CONFIG_KEYBOARD_CLOCK_OFF;
    Ok(Setup { firmware_config, config, set2_acknowledged, flushed })
}

/// Reset the keyboard and wait for its self-test.
fn reset_keyboard(io: &impl Io) -> Result<(), Error> {
    match keyboard_command(io, KBD_RESET) {
        Ok(()) => {}
        Err(Error::Refused(_)) => return Err(Error::NoKeyboard),
        Err(e) => return Err(e),
    }
    let start = io.now_ns();
    loop {
        let left = RESET_TIMEOUT_NS.saturating_sub(io.now_ns().wrapping_sub(start));
        match read_byte(io, left) {
            Some(BAT_OK) => return Ok(()),
            Some(b) if BAT_FAILED.contains(&b) => return Err(Error::KeyboardSelfTest(b)),
            Some(_) if left > 0 => continue,
            _ => return Err(Error::Timeout("keyboard self-test after reset")),
        }
    }
}

/// Write the configuration from [`init`]: keyboard interrupt and
/// translation on. Route the interrupt first: a byte already waiting
/// raises it at once.
pub fn enable_interrupts(io: &impl Io, setup: &Setup) -> Result<(), Error> {
    write_config(io, setup.config)
}

/// Make `byte` appear in the output buffer as if the mouse had sent it
/// (raising IRQ 12 if it is enabled). For testing the mouse's interrupt
/// path.
pub fn write_aux_output(io: &impl Io, byte: u8) -> Result<(), Error> {
    command(io, CMD_WRITE_AUX_OUTPUT)?;
    write_data(io, byte)
}

/// Why the mouse could not be brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuxError {
    /// The controller has no second port, or its test failed (the reply,
    /// or `None` for no reply).
    NoPort(Option<u8>),
    /// Nothing acknowledged the reset: no mouse is plugged in.
    NoMouse,
    /// The mouse's self-test after the reset failed.
    SelfTest(u8),
    /// The mouse refused a command.
    Refused(u8),
    Controller(Error),
}

impl From<Error> for AuxError {
    fn from(e: Error) -> AuxError {
        AuxError::Controller(e)
    }
}

/// What [`init_aux`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxSetup {
    /// The ID the mouse reported after the wheel detection.
    pub id: u8,
    pub kind: crate::ps2mouse::Kind,
    /// Bytes thrown away while flushing.
    pub flushed: u32,
}

/// The next byte from the mouse, waiting up to `timeout_ns`; keyboard bytes
/// that come meanwhile are dropped.
fn read_aux_byte(io: &impl Io, timeout_ns: u64) -> Option<u8> {
    let start = io.now_ns();
    loop {
        let left = timeout_ns.saturating_sub(io.now_ns().wrapping_sub(start));
        if !wait_status(io, STATUS_OUTPUT_FULL, STATUS_OUTPUT_FULL, left) {
            return None;
        }
        let status = io.read_status();
        if status & STATUS_OUTPUT_FULL == 0 {
            continue;
        }
        let byte = io.read_data();
        if status & STATUS_AUX_DATA != 0 {
            return Some(byte);
        }
        if left == 0 {
            return None;
        }
    }
}

/// Send a byte to the mouse (through 0xD4) and wait for its
/// acknowledgement, sending it again when the mouse asks to.
pub fn aux_command(io: &impl Io, byte: u8) -> Result<(), AuxError> {
    for _ in 0..=RETRIES {
        command(io, CMD_WRITE_AUX)?;
        write_data(io, byte)?;
        match read_aux_byte(io, KEYBOARD_TIMEOUT_NS) {
            Some(ACK) => return Ok(()),
            Some(RESEND) => continue,
            _ => return Err(AuxError::Refused(byte)),
        }
    }
    Err(AuxError::Refused(byte))
}

/// Send a command and its parameter byte to the mouse.
fn aux_command_with(io: &impl Io, byte: u8, param: u8) -> Result<(), AuxError> {
    aux_command(io, byte)?;
    aux_command(io, param)
}

/// The ID the mouse reports (`GET_ID`).
fn aux_id(io: &impl Io) -> Result<u8, AuxError> {
    aux_command(io, crate::ps2mouse::GET_ID)?;
    read_aux_byte(io, KEYBOARD_TIMEOUT_NS).ok_or(AuxError::Refused(crate::ps2mouse::GET_ID))
}

/// Bring up the mouse on the second port, polled, after [`init`] and before
/// [`enable_interrupts`] (whose configuration byte gets the second port's
/// interrupt and clock on here):
///
/// 1. Disable the keyboard port, so its bytes do not mix in, and flush.
/// 2. Test the second port (0xA9, expect 0x00) and enable it (0xA8).
/// 3. Reset the mouse (0xFF: ACK, 0xAA from its self-test, then its ID).
/// 4. The IntelliMouse detection: sample rates 200, 100, 80, then the ID
///    (3: a wheel); if so, 200, 200, 80 and the ID again (4: five
///    buttons).
/// 5. Sample rate 100, resolution 8 counts/mm, scaling 1:1, enable
///    reporting (0xF4).
/// 6. Enable the keyboard port again.
///
/// On failure the second port is disabled again and `setup` is left as
/// it was; the keyboard port is enabled either way.
pub fn init_aux(io: &impl Io, setup: &mut Setup) -> Result<AuxSetup, AuxError> {
    command(io, CMD_DISABLE_KEYBOARD)?;
    let flushed = flush(io);
    let result = bring_up_aux(io);
    if result.is_err() {
        let _ = command(io, CMD_DISABLE_AUX);
    }
    command(io, CMD_ENABLE_KEYBOARD)?;
    let (id, kind) = result?;
    setup.config = (setup.config | CONFIG_AUX_IRQ) & !CONFIG_AUX_CLOCK_OFF;
    Ok(AuxSetup { id, kind, flushed })
}

fn bring_up_aux(io: &impl Io) -> Result<(u8, crate::ps2mouse::Kind), AuxError> {
    use crate::ps2mouse::{self as m, Kind};
    command(io, CMD_TEST_AUX_PORT)?;
    match read_byte(io, CONTROLLER_TIMEOUT_NS) {
        Some(PORT_TEST_OK) => {}
        other => return Err(AuxError::NoPort(other)),
    }
    command(io, CMD_ENABLE_AUX)?;
    match aux_command(io, m::RESET) {
        Ok(()) => {}
        Err(AuxError::Refused(_)) => return Err(AuxError::NoMouse),
        Err(e) => return Err(e),
    }
    match read_aux_byte(io, RESET_TIMEOUT_NS) {
        Some(BAT_OK) => {}
        Some(b) => return Err(AuxError::SelfTest(b)),
        None => return Err(AuxError::NoMouse),
    }
    // Its ID follows the self-test result.
    let _ = read_aux_byte(io, KEYBOARD_TIMEOUT_NS);
    for rate in m::WHEEL_KNOCK {
        aux_command_with(io, m::SET_SAMPLE_RATE, rate)?;
    }
    let mut id = aux_id(io)?;
    if id == m::ID_WHEEL {
        for rate in m::FIVE_BUTTON_KNOCK {
            aux_command_with(io, m::SET_SAMPLE_RATE, rate)?;
        }
        id = aux_id(io)?;
    }
    aux_command_with(io, m::SET_SAMPLE_RATE, m::SAMPLE_RATE)?;
    aux_command_with(io, m::SET_RESOLUTION, m::RESOLUTION)?;
    aux_command(io, m::SET_SCALING_1_1)?;
    aux_command(io, m::ENABLE_REPORTING)?;
    Ok((id, Kind::from_id(id)))
}

/// Read one waiting byte from the keyboard, if there is one: for the
/// interrupt handler. A byte from the second port is read (so it does not
/// block the buffer) and returned as `Err`.
pub fn read_keyboard_byte(io: &impl Io) -> Option<Result<u8, u8>> {
    let status = io.read_status();
    if status & STATUS_OUTPUT_FULL == 0 {
        return None;
    }
    let byte = io.read_data();
    Some(if status & STATUS_AUX_DATA != 0 { Err(byte) } else { Ok(byte) })
}

#[cfg(test)]
pub(crate) mod sim;

#[cfg(test)]
mod tests;
