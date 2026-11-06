//! `poly-terminal`: Poly's own terminal emulator (ADR 0134), a native Poly
//! program and a Wayland client of the compositor (`poly-app`), so a crash
//! never takes the desktop with it.
//!
//! ```text
//! poly-terminal [--dir DIR] [-e PROGRAM [ARGS...]]
//! ```
//!
//! It opens a pseudo-terminal through the Linux personality ([`pty`]),
//! runs the user's shell on it (`$SHELL`, else bash, else `/bin/sh`), or
//! `PROGRAM` with `-e` (what the launcher does for desktop entries with
//! `Terminal=true`), in `DIR` or the home directory, and shows it with
//! `poly-vt`'s emulation in the Alpine look ([`term`]). The window closes
//! when the program ends.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod pty;
pub mod term;

use alloc::string::String;
use alloc::vec::Vec;

/// The command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub dir: Option<String>,
    /// `-e`: what to run instead of the shell.
    pub exec: Vec<String>,
}

/// Read the command line (without the program's name).
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dir" | "--working-directory" => o.dir = Some(it.next().ok_or("--dir needs a directory")?.clone()),
            "-e" | "--" => {
                o.exec = it.cloned().collect();
                if o.exec.is_empty() {
                    return Err("-e needs a program".into());
                }
                break;
            }
            other => {
                if let Some(d) = other.strip_prefix("--dir=") {
                    o.dir = Some(d.into());
                } else {
                    return Err(alloc::format!(
                        "unknown argument {other:?} (poly-terminal [--dir DIR] [-e PROGRAM ARGS...])"
                    ));
                }
            }
        }
    }
    Ok(o)
}

/// The program: the terminal's window until the program in it ends.
pub fn main() -> i32 {
    let args: Vec<String> = poly_rt::env::args_str().skip(1).map(String::from).collect();
    let o = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            poly_rt::eprintln!("poly-terminal: {e}");
            return 2;
        }
    };
    let ours: Vec<(String, String)> = poly_rt::env::vars_str().map(|(k, v)| (k.into(), v.into())).collect();
    let exists = |p: &str| poly_rt::fs::metadata(p).is_ok_and(|m| m.is_file());
    let sh = if exists("/bin/sh") { String::from("/bin/sh") } else { String::new() };
    let shell = pty::choose_shell(poly_rt::env::var("SHELL"), &exists);
    let mut app = term::TermApp::new(None, 80, 24);
    match (&shell, sh.is_empty()) {
        (Some(shell), false) => match pty::Pty::open(80, 24) {
            Ok((mut p, slave)) => {
                let program = if o.exec.is_empty() { alloc::vec![shell.clone(), "-i".into()] } else { o.exec.clone() };
                let argv = pty::shell_command(&sh, &p.pts, o.dir.as_deref(), &program);
                let env = pty::shell_env(&ours, shell, o.dir.as_deref());
                match p.spawn(&argv, &env, &slave) {
                    Ok(()) => {
                        poly_rt::println!("poly-terminal: {} on {}", program.join(" "), p.pts);
                        drop(slave);
                        app.pty = Some(p);
                    }
                    Err(e) => message(&mut app, &alloc::format!("Could not start {shell}: {e}")),
                }
            }
            Err(e) => message(&mut app, &alloc::format!("No pseudo-terminal: {e}")),
        },
        _ => message(&mut app, "No shell found (/bin/sh). This image has none; BusyBox or a root disk brings one."),
    }
    let status = poly_app::run(&mut app, "Terminal", "poly-terminal", (760, 480), (320, 200));
    if let Some(p) = &app.pty {
        p.hang_up();
        let _ = p.child_ended();
    }
    status
}

/// Show `text` in the window instead of a shell.
fn message(app: &mut term::TermApp, text: &str) {
    poly_rt::eprintln!("poly-terminal: {text}");
    app.vt.feed(alloc::format!("\x1b[1;31mpoly-terminal:\x1b[0m {text}\r\n").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| String::from(*s)).collect()
    }

    #[test]
    fn command_lines() {
        assert_eq!(parse_args(&[]).unwrap(), Options::default());
        let o = parse_args(&args(&["--dir", "/tmp", "-e", "vim", "-n", "x"])).unwrap();
        assert_eq!(o.dir.as_deref(), Some("/tmp"));
        assert_eq!(o.exec, vec!["vim", "-n", "x"]);
        assert_eq!(parse_args(&args(&["--dir=/a"])).unwrap().dir.as_deref(), Some("/a"));
        assert!(parse_args(&args(&["-e"])).is_err());
        assert!(parse_args(&args(&["--frobnicate"])).is_err());
    }
}
