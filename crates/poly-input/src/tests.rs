//! Layouts, dead keys and the whole byte-to-character path.

use crate::layout::{DE_CH, LAYOUTS, US};
use crate::set1::encode;
use crate::*;
use std::string::String;
use std::vec::Vec;

/// One step of a typing script.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// Press and release.
    Tap(Key),
    /// Press and release with these keys held.
    With(&'static [Key], Key),
    Down(Key),
    Up(Key),
}
use Step::*;

/// Feed the set 1 bytes for `script` and collect the outputs.
fn outputs(kb: &mut Keyboard, script: &[Step]) -> Vec<Output> {
    let mut out = Vec::new();
    let mut send = |kb: &mut Keyboard, key, pressed| {
        for &b in encode(key, pressed).iter() {
            if let Some(input) = kb.feed(b) {
                out.push(input.output);
            }
        }
    };
    for &step in script {
        match step {
            Tap(k) => {
                send(kb, k, true);
                send(kb, k, false);
            }
            With(held, k) => {
                held.iter().for_each(|&h| send(kb, h, true));
                send(kb, k, true);
                send(kb, k, false);
                held.iter().rev().for_each(|&h| send(kb, h, false));
            }
            Down(k) => send(kb, k, true),
            Up(k) => send(kb, k, false),
        }
    }
    out
}

/// What `script` types: characters as they are, special keys as `<Name>`.
fn typed_on(kb: &mut Keyboard, script: &[Step]) -> String {
    let mut s = String::new();
    for o in outputs(kb, script) {
        match o {
            Output::Special(sp) => s.push_str(&std::format!("<{sp:?}>")),
            o => s.extend(o.chars()),
        }
    }
    s
}

fn typed(layout: &'static Layout, script: &[Step]) -> String {
    typed_on(&mut Keyboard::new(layout), script)
}

fn taps(keys: &[Key]) -> Vec<Step> {
    keys.iter().map(|&k| Tap(k)).collect()
}

const SHIFT: &[Key] = &[Key::ShiftLeft];
const ALT_GR: &[Key] = &[Key::AltRight];
const CTRL: &[Key] = &[Key::ControlLeft];

/// The keys that type characters, row by row, as on the keycaps.
const TYPING_KEYS: [Key; 49] = {
    use Key::*;
    [
        Backquote,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        Digit0,
        Minus,
        Equal,
        Q,
        W,
        E,
        R,
        T,
        Y,
        U,
        I,
        O,
        P,
        BracketLeft,
        BracketRight,
        Backslash,
        A,
        S,
        D,
        F,
        G,
        H,
        J,
        K,
        L,
        Semicolon,
        Quote,
        IntlBackslash,
        Z,
        X,
        C,
        V,
        B,
        N,
        M,
        Comma,
        Period,
        Slash,
        Space,
    ]
};

fn shifted(keys: &[Key]) -> Vec<Step> {
    keys.iter().map(|&k| With(SHIFT, k)).collect()
}

// --- US ---------------------------------------------------------------------

#[test]
fn us_types_every_key_unshifted_and_shifted() {
    assert_eq!(typed(&US, &taps(&TYPING_KEYS)), "`1234567890-=qwertyuiop[]\\asdfghjkl;'\\zxcvbnm,./ ");
    assert_eq!(typed(&US, &shifted(&TYPING_KEYS)), "~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:\"|ZXCVBNM<>? ");
}

#[test]
fn caps_lock_shifts_letters_only() {
    let mut kb = Keyboard::new(&US);
    let mut script = std::vec![Tap(Key::CapsLock)];
    script.extend(taps(&[Key::A, Key::Digit1, Key::Semicolon]));
    script.extend(shifted(&[Key::A, Key::Digit1]));
    script.push(Tap(Key::CapsLock));
    script.push(Tap(Key::A));
    assert_eq!(typed_on(&mut kb, &script), "A1;a!a");
    assert_eq!(kb.leds(), Leds::default());
}

#[test]
fn right_alt_is_alt_on_us() {
    let mut kb = Keyboard::new(&US);
    let mut events = Vec::new();
    for b in [0xe0, 0x38, 0x1e] {
        events.extend(kb.feed(b));
    }
    let a = events.last().unwrap();
    assert_eq!(a.output, Output::Char('a'));
    assert_eq!(a.event.modifiers, Modifiers::ALT);
}

#[test]
fn ctrl_with_a_letter_types_its_control_character() {
    assert_eq!(typed(&US, &[With(CTRL, Key::C), With(CTRL, Key::A), With(CTRL, Key::Z)]), "\u{3}\u{1}\u{1a}");
    assert_eq!(typed(&US, &[With(&[Key::ControlRight, Key::ShiftLeft], Key::C)]), "\u{3}");
    // Ctrl with something else types it as usual.
    assert_eq!(typed(&US, &[With(CTRL, Key::Digit1), With(CTRL, Key::Space)]), "1 ");
    // The control character follows the layout: Ctrl-Z is the key that types z.
    assert_eq!(typed(&DE_CH, &[With(CTRL, Key::Y)]), "\u{1a}");
}

#[test]
fn special_keys() {
    use Key::*;
    let keys = [
        Enter,
        NumpadEnter,
        Tab,
        Backspace,
        Escape,
        Insert,
        Delete,
        Home,
        End,
        PageUp,
        PageDown,
        ArrowUp,
        ArrowDown,
        ArrowLeft,
        ArrowRight,
        F1,
        F12,
        PrintScreen,
        Pause,
        ContextMenu,
    ];
    assert_eq!(
        typed(&US, &taps(&keys)),
        "<Enter><Enter><Tab><Backspace><Escape><Insert><Delete><Home><End><PageUp><PageDown><Up><Down><Left><Right>\
         <Function(1)><Function(12)><PrintScreen><Pause><Menu>"
    );
    let mut kb = Keyboard::new(&US);
    let f = outputs(&mut kb, &taps(&[F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12]));
    let numbers: Vec<u8> = f
        .iter()
        .filter(|&&o| o != Output::None)
        .map(|o| if let Output::Special(Special::Function(n)) = o { *n } else { 0 })
        .collect();
    assert_eq!(numbers, (1..=12).collect::<Vec<u8>>());
}

#[test]
fn modifiers_locks_and_releases_type_nothing() {
    use Key::*;
    let mut kb = Keyboard::new(&US);
    let script = [
        ShiftLeft,
        ShiftRight,
        ControlLeft,
        ControlRight,
        AltLeft,
        AltRight,
        MetaLeft,
        MetaRight,
        CapsLock,
        NumLock,
        ScrollLock,
    ];
    let out = outputs(&mut kb, &taps(&script));
    assert_eq!(out.len(), 2 * script.len());
    assert!(out.iter().all(|&o| o == Output::None), "{out:?}");
    assert_eq!(kb.leds(), Leds(0b111));
}

#[test]
fn keypad_follows_num_lock_and_shift() {
    use Key::*;
    let pad = [Numpad7, Numpad8, Numpad9, Numpad4, Numpad5, Numpad6, Numpad1, Numpad2, Numpad3, Numpad0, NumpadDecimal];
    let ops = [NumpadDivide, NumpadMultiply, NumpadSubtract, NumpadAdd];
    let mut kb = Keyboard::new(&US);
    // Num Lock off: the navigation keys; 5 types nothing.
    assert_eq!(typed_on(&mut kb, &taps(&pad)), "<Home><Up><PageUp><Left><Right><End><Down><PageDown><Insert><Delete>");
    assert_eq!(typed_on(&mut kb, &taps(&ops)), "/*-+");
    kb.set_locks(Modifiers::NUM_LOCK);
    assert_eq!(typed_on(&mut kb, &taps(&pad)), "7894561230.");
    assert_eq!(typed_on(&mut kb, &taps(&ops)), "/*-+");
    // Shift reverses Num Lock.
    assert_eq!(typed_on(&mut kb, &[With(SHIFT, Numpad8), With(SHIFT, NumpadDecimal)]), "<Up><Delete>");
    assert_eq!(typed_on(&mut kb, &[Tap(NumLock), With(SHIFT, Numpad8)]), "8");
}

#[test]
fn auto_repeat_types_again() {
    let script = [Down(Key::A), Down(Key::A), Down(Key::A), Up(Key::A), Down(Key::Backspace), Down(Key::Backspace)];
    assert_eq!(typed(&US, &script), "aaa<Backspace><Backspace>");
}

// --- Swiss German -------------------------------------------------------------

#[test]
fn de_ch_types_every_key_unshifted_and_shifted() {
    // Equal is a dead key (tested below); everything else types at once.
    let keys: Vec<Key> = TYPING_KEYS.iter().copied().filter(|&k| k != Key::Equal && k != Key::BracketRight).collect();
    assert_eq!(typed(&DE_CH, &taps(&keys)), "§1234567890'qwertzuiopü$asdfghjklöä<yxcvbnm,.- ");
    assert_eq!(typed(&DE_CH, &shifted(&keys)), "°+\"*ç%&/()=?QWERTZUIOPè£ASDFGHJKLéà>YXCVBNM;:_ ");
    assert_eq!(typed(&DE_CH, &[With(SHIFT, Key::BracketRight)]), "!");
}

#[test]
fn de_ch_alt_gr() {
    use Key::*;
    let keys =
        [Digit1, Digit2, Digit3, Digit6, Digit7, Digit8, E, BracketLeft, BracketRight, Quote, Backslash, IntlBackslash];
    let script: Vec<Step> = keys.iter().map(|&k| With(ALT_GR, k)).collect();
    assert_eq!(typed(&DE_CH, &script), "¦@#¬|¢€[]{}\\");
    // Keys with nothing on the AltGr level type nothing.
    assert_eq!(typed(&DE_CH, &[With(ALT_GR, A), With(ALT_GR, Digit4)]), "");
    // AltGr is not Alt.
    let mut kb = Keyboard::new(&DE_CH);
    let ev: Vec<Input> = [0xe0, 0x38, 0x03].iter().filter_map(|&b| kb.feed(b)).collect();
    assert_eq!(ev[1].event.modifiers, Modifiers::ALT_GR);
    assert_eq!(ev[1].output, Output::Char('@'));
    // AltGr with Ctrl held still types the AltGr level.
    assert_eq!(typed(&DE_CH, &[With(&[Key::ControlLeft, Key::AltRight], Key::E)]), "€");
}

#[test]
fn de_ch_umlauts_with_shift_and_caps_lock() {
    use Key::*;
    let umlauts = [BracketLeft, Semicolon, Quote];
    assert_eq!(typed(&DE_CH, &taps(&umlauts)), "üöä");
    assert_eq!(typed(&DE_CH, &shifted(&umlauts)), "èéà");
    let mut kb = Keyboard::new(&DE_CH);
    let mut script = std::vec![Tap(CapsLock)];
    script.extend(taps(&umlauts));
    script.extend(shifted(&umlauts));
    script.extend(taps(&[A, Y]));
    script.extend(shifted(&[A]));
    assert_eq!(typed_on(&mut kb, &script), "ÜÖÄèéàAZa");
}

/// The dead keys of de-CH: ^, ` (Shift), ~ (AltGr), ´ (AltGr on Minus), ¨.
const CIRCUMFLEX: Step = Tap(Key::Equal);
const GRAVE: Step = With(SHIFT, Key::Equal);
const TILDE: Step = With(ALT_GR, Key::Equal);
const ACUTE: Step = With(ALT_GR, Key::Minus);
const DIAERESIS: Step = Tap(Key::BracketRight);

#[test]
fn de_ch_dead_keys_compose() {
    use Key::*;
    let cases: &[(&[Step], &str)] = &[
        (&[CIRCUMFLEX, Tap(A)], "â"),
        (&[CIRCUMFLEX, Tap(E)], "ê"),
        (&[CIRCUMFLEX, With(SHIFT, O)], "Ô"),
        (&[GRAVE, Tap(E)], "è"),
        (&[GRAVE, With(SHIFT, A)], "À"),
        (&[ACUTE, Tap(E)], "é"),
        (&[ACUTE, Tap(Z)], "ý"),
        (&[DIAERESIS, Tap(U)], "ü"),
        (&[DIAERESIS, Tap(Z)], "ÿ"),
        (&[DIAERESIS, With(SHIFT, I)], "Ï"),
        (&[TILDE, Tap(N)], "ñ"),
        (&[TILDE, With(SHIFT, N)], "Ñ"),
        (&[TILDE, Tap(O)], "õ"),
    ];
    for &(script, want) in cases {
        assert_eq!(typed(&DE_CH, script), want, "{script:?}");
    }
}

#[test]
fn de_ch_dead_keys_with_space_or_a_letter_that_does_not_combine() {
    use Key::*;
    let cases: &[(&[Step], &str)] = &[
        (&[CIRCUMFLEX, Tap(Space)], "^"),
        (&[GRAVE, Tap(Space)], "`"),
        (&[TILDE, Tap(Space)], "~"),
        (&[ACUTE, Tap(Space)], "´"),
        (&[DIAERESIS, Tap(Space)], "¨"),
        (&[CIRCUMFLEX, Tap(X)], "^x"),
        (&[TILDE, Tap(E)], "~e"),
        (&[DIAERESIS, Tap(Digit1)], "¨1"),
        (&[ACUTE, With(SHIFT, Digit2)], "´\""),
        // Two dead keys: both accents.
        (&[CIRCUMFLEX, CIRCUMFLEX], "^^"),
        (&[CIRCUMFLEX, DIAERESIS, Tap(A)], "^¨a"),
        // A special key cancels the accent.
        (&[CIRCUMFLEX, Tap(Backspace), Tap(A)], "<Backspace>a"),
        (&[TILDE, Tap(Escape), Tap(N)], "<Escape>n"),
    ];
    for &(script, want) in cases {
        assert_eq!(typed(&DE_CH, script), want, "{script:?}");
    }
}

#[test]
fn a_dead_key_waits_through_modifiers_and_ignores_its_repeat() {
    let mut kb = Keyboard::new(&DE_CH);
    let out = outputs(&mut kb, &[Down(Key::Equal), Down(Key::Equal), Up(Key::Equal)]);
    assert_eq!(out, [Output::Dead(DeadKey::Circumflex), Output::None, Output::None]);
    assert_eq!(kb.pending_dead_key(), Some(DeadKey::Circumflex));
    assert_eq!(typed_on(&mut kb, &[Down(Key::ShiftLeft), Tap(Key::A), Up(Key::ShiftLeft)]), "Â");
    assert_eq!(kb.pending_dead_key(), None);
}

#[test]
fn ctrl_drops_a_pending_dead_key() {
    assert_eq!(typed(&DE_CH, &[CIRCUMFLEX, With(CTRL, Key::C), Tap(Key::A)]), "\u{3}a");
}

#[test]
fn switching_layouts() {
    let mut kb = Keyboard::new(&DE_CH);
    assert_eq!(typed_on(&mut kb, &[CIRCUMFLEX]), "");
    kb.set_layout(&US);
    assert_eq!(kb.pending_dead_key(), None);
    assert_eq!(typed_on(&mut kb, &[Tap(Key::Y), With(ALT_GR, Key::A)]), "ya");
    assert_eq!(kb.layout().name, "us");
    kb.set_layout(&DE_CH);
    assert_eq!(typed_on(&mut kb, &[Tap(Key::Y), With(ALT_GR, Key::Digit2)]), "z@");
}

#[test]
fn layouts_by_name() {
    assert_eq!(layout::by_name("us").map(|l| l.name), Some("us"));
    assert_eq!(layout::by_name("DE-CH").map(|l| l.name), Some("de-ch"));
    assert!(layout::by_name("fr").is_none());
    assert_eq!(LAYOUTS[0].name, "us", "the default comes first");
}

#[test]
fn every_layout_types_something_on_every_typing_key() {
    for layout in LAYOUTS {
        for key in TYPING_KEYS {
            let d = layout.get(key);
            assert_ne!(d.base, layout::Sym::None, "{}: {key:?}", layout.name);
            assert_ne!(d.shift, layout::Sym::None, "{}: {key:?}", layout.name);
        }
        // Keys that never type text are left out of layouts.
        for key in [Key::Enter, Key::ShiftLeft, Key::F1, Key::Numpad1, Key::ArrowUp] {
            assert_eq!(*layout.get(key), layout::KeyDef::NONE, "{}: {key:?}", layout.name);
        }
    }
}

#[test]
fn every_composition_is_the_precomposed_character() {
    // Each accented letter is one Unicode scalar whose canonical
    // decomposition is the base letter and the combining accent; check a
    // few against their code points to catch a typo in the tables.
    assert_eq!(DeadKey::Circumflex.compose('a'), Some('\u{e2}'));
    assert_eq!(DeadKey::Grave.compose('E'), Some('\u{c8}'));
    assert_eq!(DeadKey::Acute.compose('y'), Some('\u{fd}'));
    assert_eq!(DeadKey::Diaeresis.compose('Y'), Some('\u{178}'));
    assert_eq!(DeadKey::Tilde.compose('N'), Some('\u{d1}'));
    for d in [DeadKey::Grave, DeadKey::Acute, DeadKey::Circumflex, DeadKey::Tilde, DeadKey::Diaeresis] {
        assert_eq!(d.compose(' '), None);
        assert_eq!(d.compose('x'), None);
        for base in 'a'..='z' {
            if let Some(c) = d.compose(base) {
                assert_eq!(d.compose(base.to_ascii_uppercase()), c.to_uppercase().next(), "{d:?} {base}");
                assert!(!c.is_ascii());
            }
        }
    }
}

// --- The whole path ---------------------------------------------------------

#[test]
fn bytes_to_text() {
    let mut kb = Keyboard::new(&US);
    // Shift-P, o, l, y, Enter.
    let bytes = [0x2a, 0x19, 0x99, 0xaa, 0x18, 0x98, 0x26, 0xa6, 0x15, 0x95, 0x1c, 0x9c];
    let inputs: Vec<Input> = bytes.iter().filter_map(|&b| kb.feed(b)).collect();
    assert_eq!(inputs.len(), bytes.len());
    let text: String = inputs.iter().flat_map(|i| i.output.chars()).collect();
    assert_eq!(text, "Poly");
    assert_eq!(inputs[1].event, KeyEvent { key: Key::P, pressed: true, repeat: false, modifiers: Modifiers::SHIFT });
    assert_eq!(inputs[10].output, Output::Special(Special::Enter));
}

#[test]
fn caps_lock_lights_its_led_and_resync_keeps_locks() {
    let mut kb = Keyboard::new(&US);
    kb.feed(0x3a);
    kb.feed(0xba);
    assert_eq!(kb.leds(), Leds::CAPS_LOCK);
    kb.feed(0x2a); // Shift down
    kb.feed(0xe0); // half a sequence
    kb.resync();
    assert_eq!(kb.modifiers(), Modifiers::CAPS_LOCK);
    assert_eq!(kb.feed(0x1e).map(|i| i.output), Some(Output::Char('A')));
}

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn random_garbage_never_panics() {
    for (seed, layout) in [(1, &US), (2, &DE_CH), (3, &US), (4, &DE_CH)] {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed);
        let mut kb = Keyboard::new(layout);
        for i in 0..200_000 {
            // Bias towards prefixes and modifiers, which have the most state.
            let byte = match rng.below(8) {
                0 => [0xe0, 0xe1, 0x2a, 0xaa, 0x38, 0x1d, 0x9d, 0x45][rng.below(8)],
                _ => rng.next() as u8,
            };
            if let Some(input) = kb.feed(byte) {
                let known = Modifiers::SHIFT
                    | Modifiers::CONTROL
                    | Modifiers::ALT
                    | Modifiers::ALT_GR
                    | Modifiers::META
                    | Modifiers::CAPS_LOCK
                    | Modifiers::NUM_LOCK
                    | Modifiers::SCROLL_LOCK;
                assert!(known.contains(input.event.modifiers));
                let _ = input.output.chars().count();
            }
            if i % 50_000 == 0 {
                kb.set_layout(LAYOUTS[rng.below(LAYOUTS.len())]);
            }
        }
    }
}

#[test]
fn random_key_streams_decode_exactly() {
    // Random presses and releases (with repeats), encoded and decoded: the
    // events come back one for one, and the modifiers match a model of
    // which keys are down.
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut kb = Keyboard::new(&DE_CH);
    let mut down = [false; Key::COUNT];
    let mut locks = Modifiers::NONE;
    for _ in 0..50_000 {
        let key = Key::ALL[rng.below(Key::COUNT)];
        let pressed = key != Key::Pause && rng.below(2) == 0;
        let mut got = Vec::new();
        for &b in encode(key, pressed).iter() {
            got.extend(kb.feed(b));
        }
        let repeat = pressed && down[key as usize];
        if pressed && !repeat {
            locks = Modifiers(
                locks.0
                    ^ match key {
                        Key::CapsLock => Modifiers::CAPS_LOCK.0,
                        Key::NumLock => Modifiers::NUM_LOCK.0,
                        Key::ScrollLock => Modifiers::SCROLL_LOCK.0,
                        _ => 0,
                    },
            );
        }
        down[key as usize] = pressed;
        let is = |k: Key| down[k as usize];
        let mut want = locks;
        for (a, b, bit) in [
            (Key::ShiftLeft, Key::ShiftRight, Modifiers::SHIFT),
            (Key::ControlLeft, Key::ControlRight, Modifiers::CONTROL),
            (Key::AltLeft, Key::AltLeft, Modifiers::ALT),
            (Key::AltRight, Key::AltRight, Modifiers::ALT_GR),
            (Key::MetaLeft, Key::MetaRight, Modifiers::META),
        ] {
            if is(a) || is(b) {
                want = want | bit;
            }
        }
        assert_eq!(got.len(), 1, "{key:?} {pressed}");
        let e = got[0].event;
        assert_eq!((e.key, e.pressed, e.repeat, e.modifiers), (key, pressed, repeat, want));
    }
}
