extern crate std;

use super::sim::{Op, Sim};
use super::*;
use std::vec;
use std::vec::Vec;

/// Fail the test if a run took longer than `limit` of simulated time.
fn quick(sim: &Sim, limit_ns: u64) {
    assert!(sim.elapsed_ns() <= limit_ns, "took {} ms of simulated time", sim.elapsed_ns() / 1_000_000);
}

#[test]
fn brings_up_a_healthy_controller_in_the_documented_order() {
    let sim = Sim::new();
    let setup = init(&sim).unwrap();
    let firmware = CONFIG_KEYBOARD_IRQ | CONFIG_SYSTEM_FLAG | CONFIG_TRANSLATE;
    let quiet = CONFIG_SYSTEM_FLAG;
    // Read after disabling both ports, which stops their clocks.
    assert_eq!(setup.firmware_config, firmware | CONFIG_KEYBOARD_CLOCK_OFF | CONFIG_AUX_CLOCK_OFF);
    assert!(setup.set2_acknowledged);
    assert_eq!(setup.flushed, 0);
    use Op::*;
    assert_eq!(
        sim.log(),
        vec![
            Command(CMD_DISABLE_KEYBOARD),
            Command(CMD_DISABLE_AUX),
            Command(CMD_READ_CONFIG),
            Command(CMD_WRITE_CONFIG),
            // The firmware's setting, both interrupts and translation off;
            // DISABLE_KEYBOARD and DISABLE_AUX stopped both clocks.
            Data(quiet | CONFIG_KEYBOARD_CLOCK_OFF | CONFIG_AUX_CLOCK_OFF),
            Command(CMD_SELF_TEST),
            Command(CMD_WRITE_CONFIG),
            Data(quiet | CONFIG_KEYBOARD_CLOCK_OFF | CONFIG_AUX_CLOCK_OFF),
            Command(CMD_TEST_KEYBOARD_PORT),
            Command(CMD_ENABLE_KEYBOARD),
            Data(KBD_RESET),
            Data(KBD_SCANCODE_SET),
            Data(2),
            Data(KBD_ENABLE_SCANNING),
        ]
    );
    // Polled so far: no interrupt while the replies came in.
    assert_eq!(sim.config() & (CONFIG_KEYBOARD_IRQ | CONFIG_AUX_IRQ | CONFIG_TRANSLATE), 0);
    sim.keyboard(|k| assert!(k.scanning && k.set == 2));

    enable_interrupts(&sim, &setup).unwrap();
    let c = sim.config();
    assert_eq!(c & (CONFIG_KEYBOARD_IRQ | CONFIG_TRANSLATE), CONFIG_KEYBOARD_IRQ | CONFIG_TRANSLATE);
    assert_eq!(c & (CONFIG_AUX_IRQ | CONFIG_KEYBOARD_CLOCK_OFF), 0, "mouse interrupt off, keyboard clock on");
    assert_ne!(c & CONFIG_AUX_CLOCK_OFF, 0, "the second port stays disabled");
    assert_ne!(c & CONFIG_SYSTEM_FLAG, 0, "the system flag is kept");
    assert!(!sim.keyboard_irq());
    sim.type_bytes(&[0x1e]);
    assert!(sim.keyboard_irq());
    assert_eq!(read_keyboard_byte(&sim), Some(Ok(0x1e)));
    assert_eq!(read_keyboard_byte(&sim), None);
    quick(&sim, 10_000_000);
}

#[test]
fn bytes_left_in_the_buffer_are_flushed_not_taken_as_replies() {
    // A stale 0x55 must not pass for the self-test's answer, nor 0xFA for an
    // acknowledgement.
    let sim = Sim::with(|s| s.output.extend([(0x55, false), (0xfa, false), (0x12, true)]));
    let setup = init(&sim).unwrap();
    assert_eq!(setup.flushed, 3);
}

#[test]
fn a_missing_controller_is_reported_at_once() {
    let sim = Sim::with(|s| s.absent = true);
    assert_eq!(init(&sim), Err(Error::NoController));
    assert!(sim.log().is_empty());
    quick(&sim, 0);
}

#[test]
fn a_controller_that_never_answers_times_out() {
    let sim = Sim::with(|s| s.mute = true);
    assert_eq!(init(&sim), Err(Error::Timeout("reading the configuration byte")));
    quick(&sim, 2 * CONTROLLER_TIMEOUT_NS);
}

#[test]
fn a_controller_that_never_takes_a_byte_times_out() {
    let sim = Sim::with(|s| s.busy_reads = u32::MAX);
    assert_eq!(init(&sim), Err(Error::Timeout("controller command")));
    assert_eq!(sim.log().len(), 1, "gave up after the first command");
    quick(&sim, 2 * CONTROLLER_TIMEOUT_NS);
}

#[test]
fn a_slow_controller_is_waited_for() {
    let sim = Sim::with(|s| s.busy_reads = 50);
    assert!(init(&sim).is_ok());
}

#[test]
fn self_test_and_port_test_failures_are_reported() {
    let sim = Sim::with(|s| s.self_test_reply = 0xfc);
    assert_eq!(init(&sim), Err(Error::SelfTest(0xfc)));
    let sim = Sim::with(|s| s.port_test_reply = 1);
    assert_eq!(init(&sim), Err(Error::PortTest(1)));
    assert!(!sim.log().contains(&Op::Command(CMD_ENABLE_KEYBOARD)), "a broken port stays disabled");
}

#[test]
fn the_configuration_is_written_again_after_the_self_test() {
    // Some controllers reset themselves in the self-test.
    let sim = Sim::with(|s| s.config_after_self_test = Some(CONFIG_KEYBOARD_IRQ | CONFIG_AUX_IRQ | CONFIG_TRANSLATE));
    let setup = init(&sim).unwrap();
    assert_eq!(sim.config() & (CONFIG_KEYBOARD_IRQ | CONFIG_AUX_IRQ | CONFIG_TRANSLATE), 0);
    assert_eq!(setup.config & CONFIG_AUX_IRQ, 0);
}

#[test]
fn no_keyboard_and_a_failing_keyboard_are_reported() {
    let sim = Sim::with(|s| s.keyboard = None);
    assert_eq!(init(&sim), Err(Error::NoKeyboard));
    // Each try waits for the acknowledgement once.
    quick(&sim, (u64::from(RETRIES) + 2) * KEYBOARD_TIMEOUT_NS);

    let sim = Sim::new();
    sim.keyboard(|k| k.bat = 0xfc);
    assert_eq!(init(&sim), Err(Error::KeyboardSelfTest(0xfc)));
}

#[test]
fn commands_are_sent_again_when_the_keyboard_asks() {
    let sim = Sim::new();
    sim.keyboard(|k| k.resend_first = 2);
    assert!(init(&sim).is_ok());
    let resets = sim.log().iter().filter(|&&op| op == Op::Data(KBD_RESET)).count();
    assert_eq!(resets, 3);

    let sim = Sim::new();
    sim.keyboard(|k| k.refuse = vec![KBD_ENABLE_SCANNING]);
    assert_eq!(init(&sim), Err(Error::Refused(KBD_ENABLE_SCANNING)));
    let tries = sim.log().iter().filter(|&&op| op == Op::Data(KBD_ENABLE_SCANNING)).count();
    assert_eq!(tries as u32, RETRIES + 1);
}

#[test]
fn a_keyboard_that_refuses_set_2_still_works() {
    let sim = Sim::new();
    sim.keyboard(|k| k.refuse = vec![KBD_SCANCODE_SET]);
    let setup = init(&sim).unwrap();
    assert!(!setup.set2_acknowledged);
    sim.keyboard(|k| assert!(k.scanning));
}

#[test]
fn keys_typed_during_bring_up_are_skipped() {
    let sim = Sim::new();
    sim.keyboard(|k| k.stray = vec![0x1e, 0x9e]);
    assert!(init(&sim).is_ok());
}

#[test]
fn bytes_from_the_second_port_are_not_keyboard_bytes() {
    let sim = Sim::new();
    let setup = init(&sim).unwrap();
    enable_interrupts(&sim, &setup).unwrap();
    sim.aux_byte(0x08);
    assert_eq!(read_keyboard_byte(&sim), Some(Err(0x08)));
}

#[test]
fn controller_can_fake_keyboard_bytes() {
    let sim = Sim::new();
    let setup = init(&sim).unwrap();
    enable_interrupts(&sim, &setup).unwrap();
    for b in [0x19, 0x99] {
        write_keyboard_output(&sim, b).unwrap();
        assert!(sim.keyboard_irq());
        assert_eq!(read_keyboard_byte(&sim), Some(Ok(b)));
    }
}

#[test]
fn leds_and_echo_are_keyboard_commands() {
    let sim = Sim::new();
    init(&sim).unwrap();
    keyboard_command(&sim, KBD_SET_LEDS).unwrap();
    keyboard_command(&sim, 0b101).unwrap();
    sim.keyboard(|k| assert_eq!(k.leds, 0b101));
    write_data(&sim, KBD_ECHO).unwrap();
    assert_eq!(read_byte(&sim, CONTROLLER_TIMEOUT_NS), Some(KBD_ECHO));
    let mut flushed: Vec<u8> = Vec::new();
    while let Some(Ok(b)) = read_keyboard_byte(&sim) {
        flushed.push(b);
    }
    assert!(flushed.is_empty());
}
