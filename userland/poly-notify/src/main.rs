//! `notify-send` for Poly (ADR 0117): a desktop notification through the
//! standard `org.freedesktop.Notifications` interface on the session bus,
//! with the options of libnotify's `notify-send` (its manual page is the
//! reference for the command line; no libnotify code was read):
//!
//! ```text
//! notify-send [-u low|normal|critical] [-t MS] [-a APP] [-i ICON] [-c CATEGORY]
//!             [-e] [-r ID] [-p] [-A [NAME=]LABEL]... [-w] SUMMARY [BODY]
//! ```
//!
//! The bus is `$DBUS_SESSION_BUS_ADDRESS` (`unix:path=...`), else
//! `$XDG_RUNTIME_DIR/bus`. With `-w` or actions it waits for the
//! notification to close and prints the action chosen.

use poly_dbus::message::Stream;
use poly_dbus::notifications::{self as dn, Notify, Urgency};
use poly_dbus::{Message, MessageType, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

const USAGE: &str = "usage: notify-send [-u low|normal|critical] [-t MS] [-a APP] [-i ICON] [-c CATEGORY] [-e] [-r ID] [-p] [-A [NAME=]LABEL]... [-w] SUMMARY [BODY]";

#[derive(Debug, Default, PartialEq)]
struct Options {
    notify: Notify,
    print_id: bool,
    wait: bool,
}

/// Parse the command line (short options with their value attached or
/// next, long ones with `=` or next).
fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        notify: Notify { app_name: "notify-send".into(), expire_timeout: -1, ..Notify::default() },
        ..Options::default()
    };
    let mut free = Vec::new();
    let mut it = args.iter();
    let mut n_action = 0;
    while let Some(a) = it.next() {
        if a == "--" {
            free.extend(it.by_ref().cloned());
            break;
        }
        let (key, inline): (&str, Option<String>) = if let Some(long) = a.strip_prefix("--") {
            match long.split_once('=') {
                Some((k, v)) => (k, Some(v.into())),
                None => (long, None),
            }
        } else if a.len() > 1 && a.starts_with('-') {
            let (k, rest) = a[1..].split_at(1);
            (k, (!rest.is_empty()).then(|| rest.into()))
        } else {
            free.push(a.clone());
            continue;
        };
        let takes_value = matches!(
            key,
            "u" | "urgency"
                | "t"
                | "expire-time"
                | "a"
                | "app-name"
                | "i"
                | "icon"
                | "c"
                | "category"
                | "r"
                | "replace-id"
                | "A"
                | "action"
                | "h"
                | "hint"
        );
        let value = if takes_value {
            Some(inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("-{key} needs a value\n{USAGE}"))?)
        } else {
            None
        };
        let v = value.unwrap_or_default();
        match key {
            "u" | "urgency" => {
                o.notify.urgency = match v.as_str() {
                    "low" => Urgency::Low,
                    "normal" => Urgency::Normal,
                    "critical" => Urgency::Critical,
                    _ => return Err(format!("unknown urgency {v} (low, normal, critical)")),
                }
            }
            "t" | "expire-time" => o.notify.expire_timeout = v.parse().map_err(|_| format!("bad time {v}"))?,
            "a" | "app-name" => o.notify.app_name = v,
            "i" | "icon" => o.notify.app_icon = v,
            "c" | "category" => o.notify.category = Some(v),
            "r" | "replace-id" => o.notify.replaces_id = v.parse().map_err(|_| format!("bad id {v}"))?,
            "A" | "action" => {
                let (name, label) = match v.split_once('=') {
                    Some((n, l)) => (n.to_string(), l.to_string()),
                    None => {
                        n_action += 1;
                        (format!("{}", n_action - 1), v)
                    }
                };
                o.notify.actions.push((name, label));
                o.wait = true;
            }
            "h" | "hint" => {} // Typed hints are not sent.
            "e" | "transient" => o.notify.transient = true,
            "p" | "print-id" => o.print_id = true,
            "w" | "wait" => o.wait = true,
            "?" | "help" => return Err(USAGE.into()),
            _ => return Err(format!("unknown option {a}\n{USAGE}")),
        }
    }
    let mut free = free.into_iter();
    o.notify.summary = free.next().ok_or_else(|| format!("no summary given\n{USAGE}"))?;
    o.notify.body = free.next().unwrap_or_default();
    if free.next().is_some() {
        return Err(format!("too many arguments\n{USAGE}"));
    }
    Ok(o)
}

/// The session bus's socket path.
fn bus_path() -> Result<String, String> {
    if let Ok(addr) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        // The first `unix:path=` address of a `;`-separated list.
        for a in addr.split(';') {
            if let Some(rest) = a.strip_prefix("unix:") {
                for kv in rest.split(',') {
                    if let Some(p) = kv.strip_prefix("path=") {
                        return Ok(p.into());
                    }
                }
            }
        }
        return Err(format!("no unix:path= in DBUS_SESSION_BUS_ADDRESS ({addr})"));
    }
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map_err(|_| "no session bus (DBUS_SESSION_BUS_ADDRESS and XDG_RUNTIME_DIR unset)")?;
    Ok(format!("{dir}/bus"))
}

/// A connection to the bus: authenticated, `Hello` said.
struct Bus {
    sock: UnixStream,
    stream: Stream,
    serial: u32,
}

impl Bus {
    fn connect(path: &str) -> Result<Bus, String> {
        let mut sock = UnixStream::connect(path).map_err(|e| format!("{path}: {e}"))?;
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let (auth, begin) = poly_dbus::auth::client_lines(uid);
        sock.write_all(&auth).map_err(|e| e.to_string())?;
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            if sock.read(&mut b).map_err(|e| e.to_string())? == 0 {
                return Err("the bus closed the connection while authenticating".into());
            }
            line.push(b[0]);
        }
        if !line.starts_with(b"OK ") {
            return Err(format!("the bus refused: {}", String::from_utf8_lossy(&line).trim()));
        }
        sock.write_all(&begin).map_err(|e| e.to_string())?;
        let mut bus = Bus { sock, stream: Stream::default(), serial: 0 };
        let hello = Message::method_call(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "Hello",
            vec![],
        );
        bus.call(hello)?;
        Ok(bus)
    }

    fn send(&mut self, m: &Message) -> Result<u32, String> {
        self.serial += 1;
        self.sock.write_all(&m.encode(self.serial)).map_err(|e| e.to_string())?;
        Ok(self.serial)
    }

    fn next(&mut self) -> Result<Message, String> {
        loop {
            if let Some(m) = self.stream.next_message() {
                return m.map_err(|e| e.0.to_string());
            }
            let mut buf = [0u8; 4096];
            let n = self.sock.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("the bus closed the connection".into());
            }
            self.stream.push(&buf[..n]);
        }
    }

    /// Call a method and wait for its return (or error).
    fn call(&mut self, m: Message) -> Result<Message, String> {
        let serial = self.send(&m)?;
        loop {
            let r = self.next()?;
            if r.reply_serial == Some(serial) {
                if r.kind == MessageType::Error {
                    let text = r.body.first().and_then(Value::as_str).unwrap_or("");
                    return Err(format!("{}: {text}", r.error_name.unwrap_or_default()));
                }
                return Ok(r);
            }
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let o = parse_args(args)?;
    let mut bus = Bus::connect(&bus_path()?)?;
    if o.wait {
        let rule = format!("type='signal',interface='{}'", dn::INTERFACE);
        let add = Message::method_call(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "AddMatch",
            vec![Value::Str(rule)],
        );
        bus.call(add)?;
    }
    let reply = bus.call(dn::notify_call(&o.notify))?;
    let id = reply.body.first().and_then(Value::as_u32).ok_or("Notify returned no ID")?;
    if o.print_id {
        println!("{id}");
    }
    if !o.wait {
        return Ok(());
    }
    loop {
        let m = bus.next()?;
        if m.kind != MessageType::Signal || m.body.first().and_then(Value::as_u32) != Some(id) {
            continue;
        }
        match m.member.as_deref() {
            Some("ActionInvoked") => {
                if let Some(key) = m.body.get(1).and_then(Value::as_str) {
                    println!("{key}");
                }
            }
            Some("NotificationClosed") => return Ok(()),
            _ => {}
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("notify-send: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_as_libnotify_has_them() {
        let o = parse_args(&args(&["-u", "critical", "-t5000", "--app-name=Poly", "-p", "Hello", "World"])).unwrap();
        assert_eq!(
            (o.notify.urgency, o.notify.expire_timeout, o.notify.app_name.as_str()),
            (Urgency::Critical, 5000, "Poly")
        );
        assert_eq!(
            (o.notify.summary.as_str(), o.notify.body.as_str(), o.print_id, o.wait),
            ("Hello", "World", true, false)
        );
        let o = parse_args(&args(&["-A", "open=Open it", "-A", "Later", "-e", "-r", "7", "S"])).unwrap();
        assert_eq!(o.notify.actions, [("open".into(), "Open it".into()), ("0".into(), "Later".into())]);
        assert!(o.wait && o.notify.transient);
        assert_eq!(o.notify.replaces_id, 7);
        let o = parse_args(&args(&["--", "-dash summary"])).unwrap();
        assert_eq!(o.notify.summary, "-dash summary");
        assert_eq!(o.notify.expire_timeout, -1, "the server's default");
        assert!(parse_args(&args(&[])).is_err());
        assert!(parse_args(&args(&["-u", "loud", "x"])).is_err());
        assert!(parse_args(&args(&["a", "b", "c"])).is_err());
        assert!(parse_args(&args(&["-z", "x"])).is_err());
    }
}
