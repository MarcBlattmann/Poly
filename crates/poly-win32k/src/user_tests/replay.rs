//! The scripted activation tests of the sheets, replayed on the host (ADR
//! 0176): the scenarios of the visible tests of GetActiveWindow.yaml,
//! GetFocus.yaml, GetForegroundWindow.yaml and ShowWindow.yaml that are
//! written as steps (`SCRIPTS` in the test programs) run against a `Wm` in
//! memory, and what they print is compared with what Windows printed (the
//! entries' `windows.json`). The scripts and the expected output are read
//! from `specs/windows/`; the runner below follows the test programs'
//! (their `run_script`, `after` and `zlist`), with this crate's functions
//! where they call `user32`.

use super::*;
use alloc::vec;
use std::path::PathBuf;

thread_local! {
    /// The window that was in the foreground when the test started.
    static KEEPER_WINDOW: Cell<u32> = const { Cell::new(0) };
}

fn info_of(h: u64) -> Option<WinInfo> {
    match env().call(Request::Window { hwnd: h }) {
        Reply::Window { info } => Some(info),
        _ => None,
    }
}

fn userdata(h: u64) -> u64 {
    match env().call(Request::GetLong { hwnd: h, index: -21, size: 8 }) {
        Reply::Value { value } => value,
        _ => 0,
    }
}

/// A window as the tests print it (`W`; with `keeper`, `Wk`): "null", the
/// tag letter, "K", "ours", "other" or "invalid".
fn name(h: u64, keeper: bool) -> String {
    if h == 0 {
        return "null".to_string();
    }
    if keeper && h as u32 == KEEPER_WINDOW.with(|k| k.get()) {
        return "K".to_string();
    }
    let Some(i) = info_of(h) else { return "invalid".to_string() };
    if i.pid != 7 {
        return "other".to_string();
    }
    let t = userdata(h);
    if (0x41..=0x5A).contains(&t) || (0x61..=0x7A).contains(&t) {
        (t as u8 as char).to_string()
    } else {
        "ours".to_string()
    }
}

/// The test programs' `proc_f`: logs the focus messages under the window's tag.
extern "system" fn script_proc(h: u64, msg: u32, w: u64, l: u64) -> u64 {
    if msg == WM_NCCREATE {
        let t = NEXT.with(|n| n.get());
        env().call(Request::SetLong { hwnd: h, index: -21, size: 8, value: t as u64 });
    }
    let t = tag(h);
    match msg {
        focus::WM_ACTIVATE => {
            log(format!("{t} WM_ACTIVATE state={} minimized={} other={}", w & 0xFFFF, w >> 16, name(l, true)))
        }
        focus::WM_SETFOCUS => log(format!("{t} WM_SETFOCUS other={}", name(w, true))),
        focus::WM_KILLFOCUS => log(format!("{t} WM_KILLFOCUS other={}", name(w, true))),
        focus::WM_NCACTIVATE => log(format!("{t} WM_NCACTIVATE w={w:#x} l={}", name(l, true))),
        focus::WM_ACTIVATEAPP => log(format!("{t} WM_ACTIVATEAPP active={w}")),
        focus::WM_ENABLE => log(format!("{t} WM_ENABLE enabled={w}")),
        focus::WM_CANCELMODE => log(format!("{t} WM_CANCELMODE")),
        WM_SHOWWINDOW => log(format!("{t} WM_SHOWWINDOW show={w} status={l}")),
        0x0215 => log(format!("{t} WM_CAPTURECHANGED new={}", name(l, false))),
        WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {
            // SAFETY: a WINDOWPOS.
            let wp = unsafe { &*(l as *const WindowPos) };
            let n = if msg == WM_WINDOWPOSCHANGING { "WM_WINDOWPOSCHANGING" } else { "WM_WINDOWPOSCHANGED" };
            log(format!("{t} {n} pos={},{} size={},{} flags={:#x}", wp.x, wp.y, wp.cx, wp.cy, wp.flags));
        }
        _ => {}
    }
    def_window_proc(env(), h, msg, w, l)
}

/// What `user32` does when a window was made: the thread's default IME
/// window with its first one (interactive session only, as in `user32`).
pub(super) fn make_ime_window(e: &TestEnv, first: u32) {
    if !screen(e).interactive || !matches!(e.call(Request::DefaultIme { tid: 0 }), Reply::Handle { hwnd: 0 }) {
        return;
    }
    let ime: Vec<u16> = "IME".encode_utf16().collect();
    e.call(Request::RegisterClass { spec: ClassSpec { name: ime.clone(), unicode: true, ..ClassSpec::default() } });
    let spec = CreateSpec {
        class: ClassRef::Name { name: ime },
        style: WS_POPUP | WS_DISABLED | WS_CLIPSIBLINGS,
        parent: u64::from(first),
        ..CreateSpec::default()
    };
    let before = NEXT.with(|n| n.replace('?'));
    create_window(e, spec, 0, 0);
    NEXT.with(|n| n.set(before));
}

struct Run<'a> {
    e: &'a TestEnv,
    out: Vec<String>,
    wins: [u64; 26],
    show_ime: bool,
}

const SW_NAMES: [&str; 12] = [
    "SW_HIDE",
    "SW_SHOWNORMAL",
    "SW_SHOWMINIMIZED",
    "SW_SHOWMAXIMIZED",
    "SW_SHOWNOACTIVATE",
    "SW_SHOW",
    "SW_MINIMIZE",
    "SW_SHOWMINNOACTIVE",
    "SW_SHOWNA",
    "SW_RESTORE",
    "SW_SHOWDEFAULT",
    "SW_FORCEMINIMIZE",
];

impl Run<'_> {
    fn win(&self, tag: u8) -> u64 {
        if tag == b'K' {
            return u64::from(KEEPER_WINDOW.with(|k| k.get()));
        }
        if tag.is_ascii_uppercase() { self.wins[(tag - b'A') as usize] } else { 0 }
    }

    fn foreground(&self) -> u64 {
        u64::from(focus::input(self.e).foreground)
    }

    /// The test's top-level windows and K from the top of the z order down.
    fn zlist(&self) -> String {
        let mut s = String::new();
        let mut h = match self.e.call(Request::GetWindow { hwnd: u64::from(desktop(self.e)), cmd: 5 }) {
            Reply::Handle { hwnd } => hwnd,
            _ => 0,
        };
        let keeper = KEEPER_WINDOW.with(|k| k.get());
        while h != 0 {
            let i = info_of(u64::from(h)).expect("a window of the list");
            let is_keeper = h == keeper;
            let tagged = i.pid == 7 && (0x41..=0x5A).contains(&userdata(u64::from(h)));
            if is_keeper || tagged {
                s.push_str(&name(u64::from(h), true));
                if i.ex_style & WS_EX_TOPMOST != 0 {
                    s.push('^');
                }
                // IsWindowVisible: 0 for every window in session 0.
                if !(i.visible && screen(self.e).interactive) {
                    s.push('~');
                }
                if i.minimized {
                    s.push('_');
                }
                s.push(' ');
            } else if self.show_ime && i.pid == 7 {
                let owner_tag = if i.owner == 0 { 0 } else { userdata(u64::from(i.owner)) };
                if (0x41..=0x5A).contains(&owner_tag) && !i.visible {
                    s.push_str("i ");
                }
            }
            h = match self.e.call(Request::GetWindow { hwnd: u64::from(h), cmd: 2 }) {
                Reply::Handle { hwnd } => hwnd,
                _ => 0,
            };
        }
        s
    }

    fn after(&mut self, label: &str) {
        self.out.push(label.to_string());
        for l in take_log() {
            self.out.push(format!("    {l}"));
        }
        let st = focus::input(self.e);
        let (a, f, g, c) = (u64::from(st.active), u64::from(st.focus), u64::from(st.foreground), u64::from(st.capture));
        let cap = if c == 0 { String::new() } else { format!(" capture={}", name(c, true)) };
        self.out.push(format!(
            "  -> active={} focus={} fg={}{cap} z: {}",
            name(a, true),
            name(f, true),
            name(g, true),
            self.zlist()
        ));
    }

    fn destroy_all(&mut self) {
        for i in 0..26 {
            if self.wins[i] != 0 {
                destroy_window(self.e, self.wins[i]);
                self.wins[i] = 0;
            }
        }
    }

    fn script(&mut self, script: &str) {
        let e = self.e;
        let mut count = 0i32;
        for tok in script.split(' ') {
            let b = tok.as_bytes();
            if b.is_empty() {
                continue;
            }
            let label = match b[0] {
                b'|' => {
                    self.destroy_all();
                    take_log();
                    count = 0;
                    self.out.push(format!(
                        "--- all windows destroyed; the foreground window is {}",
                        name(self.foreground(), true)
                    ));
                    continue;
                }
                b'+' => {
                    let t = b[1];
                    let (mut ex, mut style, mut parent, mut child) = (0u32, WS_POPUP, 0u64, false);
                    let mut label = format!("create {}", t as char);
                    let mut i = 3;
                    while i < b.len() {
                        match b[i] {
                            b't' => {
                                ex |= WS_EX_TOPMOST;
                                label.push_str(" topmost");
                            }
                            b'v' => {
                                style |= WS_VISIBLE;
                                label.push_str(" visible");
                            }
                            b'o' => {
                                style = (style & !WS_POPUP) | 0x00CF_0000;
                                label.push_str(" overlapped");
                            }
                            b'c' => {
                                style |= WS_CAPTION | 0x0002_0000;
                                label.push_str(" caption");
                            }
                            b'n' => {
                                ex |= WS_EX_NOACTIVATE;
                                label.push_str(" no-activate");
                            }
                            b'l' => {
                                ex |= WS_EX_TOOLWINDOW;
                                label.push_str(" tool");
                            }
                            b'd' => {
                                style |= WS_DISABLED;
                                label.push_str(" disabled");
                            }
                            b'@' => {
                                i += 1;
                                parent = self.win(b[i]);
                                label.push_str(&format!(" owned by {}", b[i] as char));
                            }
                            b'>' => {
                                i += 1;
                                parent = self.win(b[i]);
                                child = true;
                                label.push_str(&format!(" child of {}", b[i] as char));
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let h = if child {
                        let st = WS_CHILD | (style & WS_VISIBLE) | (style & WS_DISABLED);
                        create_with(e, t as char, 0, "ZScript", st, 10, 10, 100, 50, parent, 1 + count as u64)
                    } else {
                        let (x, y) = (30 + 150 * (count % 6), 30 + 25 * count);
                        create_with(e, t as char, ex, "ZScript", style, x, y, 170, 110, parent, 0)
                    };
                    count += 1;
                    self.wins[(t - b'A') as usize] = h;
                    if h == 0 {
                        label.push_str(": FAILED");
                    }
                    label
                }
                b'-' => {
                    let r = destroy_window(e, self.win(b[1]));
                    self.wins[(b[1] - b'A') as usize] = 0;
                    format!("DestroyWindow({}) = {}", b[1] as char, i32::from(r))
                }
                b'*' => {
                    let h = self.win(b[1]);
                    if self.foreground() != h {
                        focus::set_foreground_window(e, h);
                    }
                    take_log();
                    format!(
                        "{} brought to the foreground (messages not shown): {}",
                        b[1] as char,
                        i32::from(self.foreground() == h)
                    )
                }
                b'!' => {
                    let r = focus::set_foreground_window(e, self.win(b[1]));
                    format!("SetForegroundWindow({}) = {}", b[1] as char, i32::from(r))
                }
                b'f' => {
                    let r = focus::set_focus(e, self.win(b[1]));
                    format!("SetFocus({}) returned {}", b[1] as char, name(r, true))
                }
                b'a' => {
                    let r = focus::set_active_window(e, self.win(b[1]));
                    format!("SetActiveWindow({}) returned {}", b[1] as char, name(r, true))
                }
                b'e' => {
                    let on = b[2] == b'1';
                    let r = focus::enable_window(e, self.win(b[1]), on);
                    format!("EnableWindow({}, {}) = {}", b[1] as char, i32::from(on), i32::from(r))
                }
                b'p' | b'P' => {
                    let (after, n): (u64, String) = match b[2] {
                        b'0' => (0, "HWND_TOP".to_string()),
                        b'1' => (1, "HWND_BOTTOM".to_string()),
                        b'2' => ((-1i64) as u64, "HWND_TOPMOST".to_string()),
                        b'3' => ((-2i64) as u64, "HWND_NOTOPMOST".to_string()),
                        c => (self.win(c), format!("after {}", c as char)),
                    };
                    let na = b[0] == b'p';
                    let flags = SWP_NOMOVE | SWP_NOSIZE | if na { SWP_NOACTIVATE } else { 0 };
                    let r = set_window_pos(e, self.win(b[1]), after, 0, 0, 0, 0, flags);
                    format!(
                        "SetWindowPos({}, {n}, SWP_NOMOVE|SWP_NOSIZE{}) = {}",
                        b[1] as char,
                        if na { "|SWP_NOACTIVATE" } else { "" },
                        i32::from(r)
                    )
                }
                b'c' => {
                    let r = focus::set_capture(e, self.win(b[1]));
                    format!("SetCapture({}) returned {}", b[1] as char, name(r, true))
                }
                b'r' => {
                    let had = focus::input(e).capture != 0;
                    focus::set_capture(e, 0);
                    format!("ReleaseCapture() = {}", i32::from(had))
                }
                b'w' => "0.3 s later".to_string(),
                b'q' => {
                    take_log();
                    continue;
                }
                c if c.is_ascii_uppercase() && b.len() >= 3 && b[1] == b'=' => {
                    let mut n = i32::from(b[2] - b'0');
                    if b.len() > 3 {
                        n = n * 10 + i32::from(b[3] - b'0');
                    }
                    let r = show_window(e, self.win(c), n);
                    format!("ShowWindow({}, {}) = {}", c as char, SW_NAMES[n as usize], i32::from(r))
                }
                _ => format!("?? {tok}"),
            };
            self.after(&label);
        }
        self.destroy_all();
        take_log();
    }
}

#[allow(clippy::too_many_arguments)]
fn create_with(
    e: &TestEnv,
    tag: char,
    ex: u32,
    class: &str,
    style: u32,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    parent: u64,
    menu: u64,
) -> u64 {
    create(e, tag, ex, class, style, x, y, cx, cy, parent, menu)
}

fn specs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../specs/windows/user32.dll")
}

/// `SCRIPTS` and `SHOW_IME` of a test program.
fn scripts_of(entry: &str, test: &str) -> (Vec<String>, bool) {
    let src = std::fs::read_to_string(specs().join(format!("{entry}.tests/{test}.rs"))).expect("the test program");
    let show_ime = src.contains("const SHOW_IME: bool = true;");
    let at = src.find("const SCRIPTS:").expect("SCRIPTS");
    let body = &src[at..at + src[at..].find("];").expect("the end of SCRIPTS")];
    let scripts = body
        .lines()
        .skip(1)
        .filter_map(|l| l.trim().strip_prefix('"')?.strip_suffix("\",").map(|s| s.to_string()))
        .collect();
    (scripts, show_ime)
}

fn base64(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// What Windows printed for a test (the entry's `windows.json`).
fn recorded(entry: &str, test: &str) -> String {
    let json = std::fs::read_to_string(specs().join(format!("{entry}.tests/windows.json"))).expect("windows.json");
    let at = json.find(&format!("\"name\": \"{test}\"")).expect("the test in windows.json");
    let key = "\"output_b64\": \"";
    let from = at + json[at..].find(key).expect("its output") + key.len();
    let b64 = &json[from..from + json[from..].find('"').expect("the end of the output")];
    String::from_utf8(base64(b64)).expect("text")
}

/// Compare what a replay printed with what Windows printed.
fn same_as_windows(test: &str, out: &[String], want: &str) {
    let want: Vec<&str> = want.lines().collect();
    for (n, (got, want)) in out.iter().zip(want.iter()).enumerate() {
        if got != want {
            let from = n.saturating_sub(14);
            panic!(
                "{test}: line {} differs\n  Poly:    {got}\n  Windows: {want}\n--- Poly's lines before it:\n{}",
                n + 1,
                out[from..n].join("\n")
            );
        }
    }
    assert_eq!(out.len(), want.len(), "{test}: the number of lines");
}

thread_local! {
    /// The WM_GETMINMAXINFO messages since the last look: how many, and the last one's values.
    static MINMAX: Cell<[i32; 9]> = const { Cell::new([0; 9]) };
}

/// sw_minmax_styles' procedure: counts WM_GETMINMAXINFO.
extern "system" fn minmax_proc(h: u64, msg: u32, w: u64, l: u64) -> u64 {
    if msg == WM_GETMINMAXINFO {
        // SAFETY: a MINMAXINFO.
        let m = unsafe { &*(l as *const MinMaxInfo) };
        let n = MINMAX.with(|c| c.get())[0];
        MINMAX.with(|c| {
            c.set([
                n + 1,
                m.max_size.x,
                m.max_size.y,
                m.max_position.x,
                m.max_position.y,
                m.min_track.x,
                m.min_track.y,
                m.max_track.x,
                m.max_track.y,
            ])
        });
    }
    def_window_proc(env(), h, msg, w, l)
}

fn minmax_seen() -> String {
    let g = MINMAX.with(|c| c.replace([0; 9]));
    if g[0] == 0 {
        "none".to_string()
    } else {
        format!(
            "x{} max {}x{} at {},{} mintrack {},{} maxtrack {},{}",
            g[0], g[1], g[2], g[3], g[4], g[5], g[6], g[7], g[8]
        )
    }
}

/// ShowWindow.yaml `tiled_styles`, `minmax_by_style`: sw_minmax_styles (session 0),
/// every style and extended style of the test, as its program prints them.
#[test]
fn sw_minmax_styles_as_on_windows() {
    with_env(|e| {
        let spec = ClassSpec {
            name: "SwMinMaxStyles".encode_utf16().collect(),
            wndproc: minmax_proc as *const () as u64,
            unicode: true,
            ..ClassSpec::default()
        };
        assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
        let mut out = vec!["ok registered".to_string()];
        let styles =
            [0, WS_BORDER, WS_DLGFRAME, WS_CAPTION, WS_THICKFRAME, WS_CAPTION | WS_THICKFRAME, 0x00CF_0000, WS_SYSMENU];
        let exs = [0, WS_EX_TOOLWINDOW, crate::metrics::WS_EX_DLGMODALFRAME, crate::metrics::WS_EX_STATICEDGE, 0x200];
        let rect = |r: Rect| format!("{},{},{},{}", r.left, r.top, r.right, r.bottom);
        for popup in [false, true] {
            for ex in exs {
                for st in styles {
                    let style = st | if popup { WS_POPUP } else { 0 };
                    MINMAX.with(|c| c.set([0; 9]));
                    let h = create(e, 'M', ex, "SwMinMaxStyles", style, 60, 70, 300, 200, 0, 0);
                    let at_create = minmax_seen();
                    let made = info_of(h).expect("the window");
                    show_window(e, h, SW_SHOWMAXIMIZED);
                    let at_max = minmax_seen();
                    let max = info_of(h).expect("the window");
                    show_window(e, h, SW_SHOWMINIMIZED);
                    let at_min = minmax_seen();
                    let min = info_of(h).expect("the window");
                    destroy_window(e, h);
                    out.push(format!(
                        "style {style:#010x} ex {ex:#x}: window style {:#010x} ex {:#x}",
                        made.style, made.ex_style
                    ));
                    out.push(format!("  created: {at_create}"));
                    out.push(format!(
                        "  maximized: {at_max}; window {} client {}x{}",
                        rect(max.window),
                        max.client.width(),
                        max.client.height()
                    ));
                    out.push(format!("  then minimized: {at_min}; window {}", rect(min.window)));
                }
            }
        }
        out.push("summary 1 passed, 0 failed".to_string());
        same_as_windows("sw_minmax_styles", &out, &recorded("ShowWindow", "sw_minmax_styles"));
    });
}

/// Run a scripted test as its program does and compare with Windows.
fn replay(entry: &str, test: &str, interactive_session: bool) {
    let (scripts, show_ime) = scripts_of(entry, test);
    assert!(!scripts.is_empty());
    with_env(|e| {
        if interactive_session {
            interactive(e);
        }
        e.ime.set(true);
        let spec = ClassSpec {
            name: "ZScript".encode_utf16().collect(),
            wndproc: script_proc as *const () as u64,
            unicode: true,
            ..ClassSpec::default()
        };
        assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
        let k = focus::input(e).foreground;
        KEEPER_WINDOW.with(|c| c.set(k));
        let mut run = Run { e, out: Vec::new(), wins: [0; 26], show_ime };
        run.out.push("ok registered".to_string());
        let other = k != 0 && info_of(u64::from(k)).is_some_and(|i| i.pid != 7);
        run.out.push(format!(
            "at the start the foreground window K is a window of another process: {}; z: {}",
            i32::from(other),
            run.zlist()
        ));
        for (i, s) in scripts.iter().enumerate() {
            run.out.push(format!("=== scenario {} (the foreground window is {})", i + 1, name(run.foreground(), true)));
            run.script(s);
        }
        run.out.push("summary 1 passed, 0 failed".to_string());
        same_as_windows(test, &run.out, &recorded(entry, test));
    });
}

#[test]
fn gaw_successor_as_on_windows() {
    replay("GetActiveWindow", "gaw_successor", true);
}

#[test]
fn gaw_never_active_as_on_windows() {
    replay("GetActiveWindow", "gaw_never_active", true);
}

#[test]
fn gaw_min_inactive_as_on_windows() {
    replay("GetActiveWindow", "gaw_min_inactive", true);
}

#[test]
fn gaw_background_a_as_on_windows() {
    replay("GetActiveWindow", "gaw_background_a", true);
}

#[test]
fn gaw_background_b_as_on_windows() {
    replay("GetActiveWindow", "gaw_background_b", true);
}

#[test]
fn gaw_background_c_as_on_windows() {
    replay("GetActiveWindow", "gaw_background_c", true);
}

#[test]
fn gf_zorder_as_on_windows() {
    replay("GetFocus", "gf_zorder", true);
}

#[test]
fn gf_background_as_on_windows() {
    replay("GetFocus", "gf_background", true);
}

#[test]
fn gfw_right_as_on_windows() {
    replay("GetForegroundWindow", "gfw_right", true);
}

#[test]
fn gfw_minimize_as_on_windows() {
    replay("GetForegroundWindow", "gfw_minimize", true);
}

#[test]
fn sw_activation_as_on_windows() {
    replay("ShowWindow", "sw_activation", false);
}

// ----- the earlier tests of the sheets (their own procedure and helpers)

/// The earlier tests' `proc_f`: the focus and position messages as their
/// `log_b` / `log_msg` print them.
extern "system" fn earlier_proc(h: u64, msg: u32, w: u64, l: u64) -> u64 {
    if msg == WM_NCCREATE {
        let t = NEXT.with(|n| n.get());
        env().call(Request::SetLong { hwnd: h, index: -21, size: 8, value: t as u64 });
    }
    let t = tag(h);
    let lo = |v: u64| v as u16 as i16;
    let hi = |v: u64| (v >> 16) as u16 as i16;
    // gaw_owner logs the focus and position messages only.
    let sizes = matches!(msg, WM_GETMINMAXINFO | WM_NCCALCSIZE | WM_SIZE | WM_MOVE | WM_DESTROY | WM_NCDESTROY);
    if sizes && FOCUS_ONLY.with(|f| f.get()) {
        return def_window_proc(env(), h, msg, w, l);
    }
    match msg {
        focus::WM_ACTIVATE => {
            log(format!("{t} WM_ACTIVATE state={} minimized={} other={}", w & 0xFFFF, w >> 16, name(l, false)))
        }
        focus::WM_ACTIVATEAPP => log(format!("{t} WM_ACTIVATEAPP active={w}")),
        focus::WM_NCACTIVATE => {
            let other = if l == 0 { "0" } else { "handle" };
            log(format!("{t} WM_NCACTIVATE active={w} l={other}"))
        }
        focus::WM_SETFOCUS => log(format!("{t} WM_SETFOCUS other={}", name(w, false))),
        focus::WM_KILLFOCUS => log(format!("{t} WM_KILLFOCUS other={}", name(w, false))),
        focus::WM_ENABLE => log(format!("{t} WM_ENABLE enabled={w}")),
        focus::WM_CANCELMODE => log(format!("{t} WM_CANCELMODE")),
        WM_SHOWWINDOW => log(format!("{t} WM_SHOWWINDOW show={w} status={l}")),
        0x0215 => log(format!("{t} WM_CAPTURECHANGED new={}", name(l, false))),
        WM_GETMINMAXINFO => {
            // SAFETY: a MINMAXINFO.
            let m = unsafe { &*(l as *const MinMaxInfo) };
            log(format!(
                "{t} WM_GETMINMAXINFO maxsize={},{} maxpos={},{} mintrack={},{} maxtrack={},{}",
                m.max_size.x,
                m.max_size.y,
                m.max_position.x,
                m.max_position.y,
                m.min_track.x,
                m.min_track.y,
                m.max_track.x,
                m.max_track.y
            ));
        }
        WM_NCCALCSIZE if w != 0 => {
            // SAFETY: NCCALCSIZE_PARAMS and its WINDOWPOS.
            let (r, wp) = unsafe {
                let p = &*(l as *const NcCalcSizeParams);
                (p.rgrc[0], &*p.pos)
            };
            log(format!(
                "{t} WM_NCCALCSIZE 1 new={},{},{},{} pos={},{},{},{} flags={:#x}",
                r.left, r.top, r.right, r.bottom, wp.x, wp.y, wp.cx, wp.cy, wp.flags
            ));
        }
        WM_NCCALCSIZE => {
            // SAFETY: a RECT.
            let r = unsafe { &*(l as *const CRect) };
            log(format!("{t} WM_NCCALCSIZE 0 rect={},{},{},{}", r.left, r.top, r.right, r.bottom));
        }
        WM_WINDOWPOSCHANGING | WM_WINDOWPOSCHANGED => {
            // SAFETY: a WINDOWPOS.
            let wp = unsafe { &*(l as *const WindowPos) };
            let n = if msg == WM_WINDOWPOSCHANGING { "WM_WINDOWPOSCHANGING" } else { "WM_WINDOWPOSCHANGED" };
            log(format!("{t} {n} pos={},{} size={},{} flags={:#x}", wp.x, wp.y, wp.cx, wp.cy, wp.flags));
        }
        WM_SIZE => log(format!("{t} WM_SIZE w={w} l={},{}", lo(l), hi(l))),
        WM_MOVE => log(format!("{t} WM_MOVE w={w} l={},{}", lo(l), hi(l))),
        WM_DESTROY => log(format!("{t} WM_DESTROY w={w:#x} l={l:#x}")),
        WM_NCDESTROY => log(format!("{t} WM_NCDESTROY w={w:#x} l={l:#x}")),
        WM_CHILDACTIVATE => log(format!("{t} WM_CHILDACTIVATE w={w:#x} l={l:#x}")),
        _ => {}
    }
    def_window_proc(env(), h, msg, w, l)
}

thread_local! {
    /// The running earlier test logs no size, move or destroy messages.
    static FOCUS_ONLY: Cell<bool> = const { Cell::new(false) };
}

/// An earlier test on the interactive desktop: its class, its lines.
struct Earlier<'a> {
    e: &'a TestEnv,
    class: &'static str,
    out: Vec<String>,
}

impl<'a> Earlier<'a> {
    fn start(e: &'a TestEnv, class: &'static str) -> Self {
        interactive(e);
        e.ime.set(true);
        let spec = ClassSpec {
            name: class.encode_utf16().collect(),
            wndproc: earlier_proc as *const () as u64,
            unicode: true,
            ..ClassSpec::default()
        };
        assert!(matches!(e.call(Request::RegisterClass { spec }), Reply::Atom { .. }));
        KEEPER_WINDOW.with(|c| c.set(0));
        Earlier { e, class, out: vec!["ok registered".to_string()] }
    }

    #[allow(clippy::too_many_arguments)]
    fn make(&self, tag: char, ex: u32, style: u32, x: i32, y: i32, cx: i32, cy: i32, parent: u64, menu: u64) -> u64 {
        create(self.e, tag, ex, self.class, style, x, y, cx, cy, parent, menu)
    }

    /// `begin`: the window in the foreground by SetForegroundWindow, the log emptied.
    fn begin(&self, h: u64) -> bool {
        if u64::from(focus::input(self.e).foreground) != h {
            focus::set_foreground_window(self.e, h);
        }
        take_log();
        u64::from(focus::input(self.e).foreground) == h
    }

    /// `acquire`: a second try after hiding and showing the window.
    fn acquire(&self, h: u64) -> bool {
        if self.begin(h) {
            return true;
        }
        show_window(self.e, h, SW_HIDE);
        show_window(self.e, h, SW_SHOW);
        self.begin(h)
    }

    fn flush(&mut self) {
        for l in take_log() {
            self.out.push(format!("  {l}"));
        }
    }

    fn focus_state(&mut self, label: &str) {
        let st = focus::input(self.e);
        self.out.push(format!(
            "{label}: active={} focus={} fg={}",
            name(u64::from(st.active), false),
            name(u64::from(st.focus), false),
            name(u64::from(st.foreground), false)
        ));
    }

    /// `report`: the log, the label, the state.
    fn report(&mut self, label: &str) {
        self.flush();
        self.out.push(label.to_string());
        self.focus_state("  state");
    }

    /// Compare with Windows; the summary line is the recorded one.
    fn check(mut self, entry: &str, test: &str) {
        let want = recorded(entry, test);
        self.out.push(want.lines().last().unwrap_or_default().to_string());
        same_as_windows(test, &self.out, &want);
    }
}

const OVERLAPPED_WINDOW: u32 = 0x00CF_0000;

/// GetFocus.yaml gf_show_popup / gf_show_overlapped: a hidden window N shown
/// with each command while A is active with the focus on its child.
fn gf_show(kind: u8, class: &'static str, test: &str) {
    with_env(|e| {
        let mut t = Earlier::start(e, class);
        for cmd in [1, 5, 4, 8, 2, 7, 3] {
            let a = t.make('A', 0, OVERLAPPED_WINDOW | WS_VISIBLE, 30, 30, 360, 260, 0, 0);
            let (style, kname) = if kind == 0 { (OVERLAPPED_WINDOW, "overlapped") } else { (WS_POPUP, "popup") };
            let n = t.make('N', 0, style, 420, 30, 300, 220, 0, 0);
            let ok = t.acquire(a);
            let ca = t.make('a', 0, WS_CHILD | WS_VISIBLE, 10, 10, 100, 60, a, 1);
            t.make('n', 0, WS_CHILD | WS_VISIBLE, 10, 10, 100, 60, n, 1);
            focus::set_focus(e, ca);
            take_log();
            t.out.push(format!("=== {kname} N / {} (A foreground: {})", SW_NAMES[cmd as usize], i32::from(ok)));
            let r = show_window(e, n, cmd);
            t.out.push(format!("  returned {}", i32::from(r)));
            t.report("  after");
            destroy_window(e, n);
            destroy_window(e, a);
            take_log();
        }
        t.check("GetFocus", test);
    });
}

#[test]
fn gf_show_popup_as_on_windows() {
    gf_show(1, "GfShowPOPUP", "gf_show_popup");
}

#[test]
fn gf_show_overlapped_as_on_windows() {
    gf_show(0, "GfShowOVERLAPPED", "gf_show_overlapped");
}

/// gaw_zhide's z list: A and B from the top down.
fn zlist_ab(e: &TestEnv) -> String {
    let mut s = String::new();
    let mut h = match e.call(Request::GetWindow { hwnd: u64::from(desktop(e)), cmd: 5 }) {
        Reply::Handle { hwnd } => hwnd,
        _ => 0,
    };
    while h != 0 {
        let i = info_of(u64::from(h)).expect("a window of the list");
        let t = userdata(u64::from(h));
        if i.pid == 7 && (t == 0x41 || t == 0x42) {
            s.push_str(&name(u64::from(h), false));
            if !i.visible {
                s.push('~');
            }
            if i.ex_style & WS_EX_TOPMOST != 0 {
                s.push('^');
            }
            s.push(' ');
        }
        h = match e.call(Request::GetWindow { hwnd: u64::from(h), cmd: 2 }) {
            Reply::Handle { hwnd } => hwnd,
            _ => 0,
        };
    }
    s
}

/// GetActiveWindow.yaml gaw_zhide: hiding or destroying the active window
/// with B in different places of the z order.
#[test]
fn gaw_zhide_as_on_windows() {
    with_env(|e| {
        let mut t = Earlier::start(e, "GawZHide");
        let top = WS_EX_TOPMOST;
        let plan: [(u32, u8, u32, bool); 10] = [
            (0, 0, 0, false),
            (top, 0, 0, false),
            (0, 1, 0, false),
            (0, 2, 0, false),
            (0, 3, 0, false),
            (0, 3, top, false),
            (top, 3, top, false),
            (top, 2, top, false),
            (0, 0, 0, true),
            (0, 3, 0, true),
        ];
        for (aex, bmode, bex, destroy) in plan {
            let a = t.make('A', aex, OVERLAPPED_WINDOW | WS_VISIBLE, 40, 40, 320, 220, 0, 0);
            let b = t.make('B', bex, OVERLAPPED_WINDOW, 420, 40, 320, 220, 0, 0);
            let ok = t.acquire(a);
            match bmode {
                0 => {}
                1 => {
                    show_window(e, b, SW_SHOWMINNOACTIVE);
                }
                2 => {
                    show_window(e, b, SW_SHOWNORMAL);
                    focus::set_foreground_window(e, a);
                }
                _ => {
                    show_window(e, b, SW_SHOWNA);
                }
            }
            take_log();
            let an = if aex & top != 0 { "topmost A" } else { "A" };
            let bn = ["no B shown", "B minimized", "B below A", "B above A (SW_SHOWNA)"][bmode as usize];
            t.out.push(format!(
                "=== {an} / {bn}{} / {} (foreground {})",
                if bex & top != 0 { " topmost" } else { "" },
                if destroy { "DestroyWindow(A)" } else { "SW_HIDE A" },
                i32::from(ok)
            ));
            t.out.push(format!("  z before: {}", zlist_ab(e)));
            if destroy {
                destroy_window(e, a);
            } else {
                show_window(e, a, SW_HIDE);
            }
            t.flush();
            t.out.push(format!("  z after: {}", zlist_ab(e)));
            t.focus_state("  state");
            if !destroy {
                destroy_window(e, a);
            }
            destroy_window(e, b);
            take_log();
        }
        t.check("GetActiveWindow", "gaw_zhide");
    });
}

/// GetActiveWindow.yaml gaw_leave_a / gaw_leave_b: the active A hidden,
/// minimized, destroyed or shown minimized with B in different states.
fn gaw_leave(class: &'static str, test: &str, plan: [(u8, i32); 8]) {
    with_env(|e| {
        let mut t = Earlier::start(e, class);
        for (bmode, cmd) in plan {
            let a = t.make('A', 0, OVERLAPPED_WINDOW | WS_VISIBLE, 40, 40, 320, 220, 0, 0);
            let b = t.make('B', 0, OVERLAPPED_WINDOW, 420, 40, 320, 220, 0, 0);
            let mut ok = t.acquire(a);
            match bmode {
                0 => {}
                1 => {
                    show_window(e, b, SW_SHOWNA);
                }
                2 => {
                    show_window(e, b, SW_SHOWMINNOACTIVE);
                }
                _ => {
                    show_window(e, b, SW_SHOWNORMAL);
                    ok = t.acquire(a);
                }
            }
            take_log();
            let bname =
                ["B not shown", "B shown (SW_SHOWNA)", "B minimized (SW_SHOWMINNOACTIVE)", "B was active before A"]
                    [bmode as usize];
            let what = if cmd < 0 { "DestroyWindow" } else { SW_NAMES[cmd as usize] };
            t.out.push(format!("=== {bname} / {what} of A (A foreground: {})", i32::from(ok)));
            t.focus_state("  before");
            let r = if cmd < 0 { destroy_window(e, a) } else { show_window(e, a, cmd) };
            t.out.push(format!("  returned {}", i32::from(r)));
            t.report("  after");
            if cmd >= 0 {
                for (label, h) in [("A", a), ("B", b)] {
                    let i = info_of(h).expect("the window");
                    t.out.push(format!(
                        "  {label}: style_visible={} iconic={} zoomed={} rect {},{},{},{}",
                        i32::from(i.style & WS_VISIBLE != 0),
                        i32::from(i.minimized),
                        i32::from(i.maximized && !i.minimized),
                        i.window.left,
                        i.window.top,
                        i.window.right,
                        i.window.bottom
                    ));
                }
                destroy_window(e, a);
            }
            destroy_window(e, b);
            take_log();
        }
        t.check("GetActiveWindow", test);
    });
}

#[test]
fn gaw_leave_a_as_on_windows() {
    gaw_leave("GawLeaveA", "gaw_leave_a", [(0, 0), (0, 6), (0, -1), (0, 2), (1, 0), (1, 6), (1, -1), (1, 2)]);
}

#[test]
fn gaw_leave_b_as_on_windows() {
    gaw_leave("GawLeaveB", "gaw_leave_b", [(3, 0), (3, 6), (3, 2), (3, -1), (2, 0), (2, 6), (2, 2), (2, -1)]);
}

/// GetActiveWindow.yaml gaw_activate_a / _b / _c: a ShowWindow command on B
/// (hidden, visible or minimized; plain or WS_EX_NOACTIVATE) while A is active.
fn gaw_activate(class: &'static str, test: &str, plan: [(u32, u8, i32); 8]) {
    with_env(|e| {
        let mut t = Earlier::start(e, class);
        for (bex, bstate, cmd) in plan {
            let a = t.make('A', 0, WS_POPUP | WS_VISIBLE, 40, 40, 300, 200, 0, 0);
            let b = t.make('B', bex, WS_POPUP, 400, 40, 300, 200, 0, 0);
            match bstate {
                0 => {}
                1 => {
                    show_window(e, b, SW_SHOWNA);
                }
                _ => {
                    show_window(e, b, SW_SHOWMINNOACTIVE);
                }
            }
            let ok = t.acquire(a);
            take_log();
            let exn = if bex == 0 { "plain" } else { "NOACTIVATE" };
            let bn = ["hidden", "visible", "minimized"][bstate as usize];
            t.out.push(format!("=== B {exn} {bn} / {} (A foreground: {})", SW_NAMES[cmd as usize], i32::from(ok)));
            let r = show_window(e, b, cmd);
            t.out.push(format!("  returned {}", i32::from(r)));
            t.report("  after");
            let i = info_of(b).expect("the window");
            t.out.push(format!(
                "  B: style_visible={} iconic={} zoomed={} rect {},{},{},{}",
                i32::from(i.style & WS_VISIBLE != 0),
                i32::from(i.minimized),
                i32::from(i.maximized && !i.minimized),
                i.window.left,
                i.window.top,
                i.window.right,
                i.window.bottom
            ));
            destroy_window(e, a);
            destroy_window(e, b);
            take_log();
        }
        t.check("GetActiveWindow", test);
    });
}

#[test]
fn gaw_activate_a_as_on_windows() {
    let plan = [(0, 0, 1), (0, 0, 5), (0, 0, 3), (0, 0, 2), (0, 0, 4), (0, 0, 8), (0, 0, 9), (0, 1, 1)];
    gaw_activate("GawActivateA", "gaw_activate_a", plan);
}

#[test]
fn gaw_activate_b_as_on_windows() {
    let plan = [(0, 1, 5), (0, 1, 3), (0, 1, 2), (0, 1, 4), (0, 1, 8), (0, 1, 9), (0, 2, 1), (0, 2, 3)];
    gaw_activate("GawActivateB", "gaw_activate_b", plan);
}

#[test]
fn gaw_activate_c_as_on_windows() {
    let n = WS_EX_NOACTIVATE;
    let plan = [(0, 2, 4), (0, 2, 8), (0, 2, 9), (n, 0, 1), (n, 0, 5), (n, 0, 8), (n, 0, 2), (n, 1, 1)];
    gaw_activate("GawActivateC", "gaw_activate_c", plan);
}

/// GetActiveWindow.yaml gaw_owner (`owned_shown`, `owned_destroyed`,
/// `owner_disabled`): popups owned by the active window shown, destroyed
/// and hidden.
#[test]
fn gaw_owner_as_on_windows() {
    with_env(|e| {
        FOCUS_ONLY.with(|f| f.set(true));
        let mut t = Earlier::start(e, "GawOwner");
        let top = WS_EX_TOPMOST;
        let a = t.make('A', top, WS_POPUP | WS_VISIBLE, 50, 50, 300, 200, 0, 0);
        let ok = t.begin(a);
        t.out.push(format!("{} foreground", if ok { "ok" } else { "not ok" }));
        let gaw = |t: &mut Earlier<'_>, label: &str| {
            t.flush();
            let active = u64::from(focus::input(e).active);
            t.out.push(format!("{label}: GetActiveWindow={}", name(active, false)));
            t.focus_state("  state");
        };
        gaw(&mut t, "start");
        let o1 = t.make('O', top, WS_POPUP, 400, 50, 200, 120, a, 0);
        show_window(e, o1, SW_SHOW);
        gaw(&mut t, "ShowWindow(O1 owned by A, SW_SHOW)");
        destroy_window(e, o1);
        gaw(&mut t, "DestroyWindow(O1)");
        let o2 = t.make('P', top, WS_POPUP, 400, 50, 200, 120, a, 0);
        show_window(e, o2, SW_SHOW);
        gaw(&mut t, "ShowWindow(O2 owned by A, SW_SHOW)");
        show_window(e, o2, SW_HIDE);
        gaw(&mut t, "ShowWindow(O2, SW_HIDE)");
        destroy_window(e, o2);
        gaw(&mut t, "DestroyWindow(O2, hidden)");
        focus::enable_window(e, a, false);
        gaw(&mut t, "EnableWindow(A, FALSE)");
        let o3 = t.make('Q', top, WS_POPUP, 400, 50, 200, 120, a, 0);
        show_window(e, o3, SW_SHOW);
        gaw(&mut t, "ShowWindow(O3 owned by disabled A, SW_SHOW)");
        destroy_window(e, o3);
        gaw(&mut t, "DestroyWindow(O3) while A is disabled");
        focus::enable_window(e, a, true);
        gaw(&mut t, "EnableWindow(A, TRUE)");
        destroy_window(e, a);
        take_log();
        FOCUS_ONLY.with(|f| f.set(false));
        t.check("GetActiveWindow", "gaw_owner");
    });
}
