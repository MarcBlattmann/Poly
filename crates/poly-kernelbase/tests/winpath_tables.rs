//! Runs the recorded results of the visible tests of the shlwapi `Path*W` and kernelbase
//! `PathCch*` spec entries (`tests/data/<Function>.txt`, one observation per line, as the
//! tests of `specs/windows/` wrote them) through `poly_kernelbase::winpath`.

use poly_kernelbase::winpath as wp;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn s(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

/// The quoted strings of a text, in order (a string never holds a quote).
fn quoted(t: &str) -> Vec<&str> {
    t.split('"').skip(1).step_by(2).collect()
}

/// The text with its quoted strings cut out.
fn unquoted(t: &str) -> String {
    t.split('"').step_by(2).collect::<Vec<_>>().join("")
}

fn num(t: &str) -> i64 {
    let t = t.trim();
    match t.strip_prefix("0x") {
        Some(h) => i64::from_str_radix(h, 16).unwrap(),
        None => t.parse().unwrap(),
    }
}

/// Runs `check(left, right)` on each line of `data`, collecting what differs.
fn run(name: &str, data: &str, check: impl Fn(&str, &str) -> Option<String>) {
    let mut bad = Vec::new();
    let mut n = 0;
    for line in data.lines().filter(|l| !l.is_empty()) {
        let rest = line.strip_prefix(name).unwrap_or(line);
        let (l, r) = rest.split_once(" -> ").unwrap_or_else(|| panic!("no arrow in {line}"));
        n += 1;
        if let Some(why) = check(l, r) {
            bad.push(format!("{line}\n    {why}"));
        }
    }
    assert!(bad.is_empty(), "{name}: {} of {n} lines differ:\n{}", bad.len(), bad.join("\n"));
}

fn eq<T: PartialEq + std::fmt::Debug>(got: T, want: T) -> Option<String> {
    if got == want { None } else { Some(format!("got {got:?}, want {want:?}")) }
}

#[test]
fn is_relative() {
    run("PathIsRelativeW", include_str!("data/PathIsRelativeW.txt"), |l, r| {
        eq(i64::from(wp::is_relative(&w(quoted(l)[0]))), num(r))
    });
}

#[test]
fn drive_number() {
    run("PathGetDriveNumberW", include_str!("data/PathGetDriveNumberW.txt"), |l, r| {
        eq(i64::from(wp::drive_number(&w(quoted(l)[0]))), num(r))
    });
}

#[test]
fn is_file_spec() {
    run("PathIsFileSpecW", include_str!("data/PathIsFileSpecW.txt"), |l, r| {
        eq(i64::from(wp::is_file_spec(&w(quoted(l)[0]))), num(r))
    });
}

#[test]
fn is_unc() {
    run("PathIsUNCW", include_str!("data/PathIsUNCW.txt"), |l, r| eq(i64::from(wp::is_unc(&w(quoted(l)[0]))), num(r)));
}

#[test]
fn is_unc_server() {
    run("PathIsUNCServerW", include_str!("data/PathIsUNCServerW.txt"), |l, r| {
        eq(i64::from(wp::is_unc_server(&w(quoted(l)[0]))), num(r))
    });
}

#[test]
fn is_unc_server_share() {
    run("PathIsUNCServerShareW", include_str!("data/PathIsUNCServerShareW.txt"), |l, r| {
        eq(i64::from(wp::is_unc_server_share(&w(quoted(l)[0]))), num(r))
    });
}

#[test]
fn is_network_path() {
    run("PathIsNetworkPathW", include_str!("data/PathIsNetworkPathW.txt"), |l, r| {
        eq(i64::from(wp::is_network_path(&w(quoted(l)[0]))), num(r))
    });
}

#[test]
fn is_root() {
    run("PathIsRootW", include_str!("data/PathIsRootW.txt"), |l, r| {
        eq(i64::from(wp::is_root(&w(quoted(l)[0]))), num(r))
    });
}

#[test]
fn strip_path() {
    run("PathStripPathW", include_str!("data/PathStripPathW.txt"), |l, r| {
        let p = w(quoted(l)[0]);
        eq(s(&p[wp::strip_path_start(&p)..]), quoted(r)[0].to_string())
    });
}

#[test]
fn get_args() {
    run("PathGetArgsW", include_str!("data/PathGetArgsW.txt"), |l, r| {
        let p = w(quoted(l)[0]);
        let at = wp::args_start(&p);
        eq(
            (at as i64, s(&p[at..])),
            (num(r.trim_start().trim_start_matches('+').split(' ').next().unwrap()), quoted(r)[0].to_string()),
        )
    });
}

#[test]
fn skip_root() {
    run("PathSkipRootW", include_str!("data/PathSkipRootW.txt"), |l, r| {
        let p = w(quoted(l)[0]);
        match wp::skip_root(&p) {
            None => eq("NULL".to_string(), r.split(' ').next().unwrap().to_string()),
            Some(at) => eq(
                (format!("+{at}"), s(&p[at..])),
                (r.split(' ').next().unwrap().to_string(), quoted(r)[0].to_string()),
            ),
        }
    });
}

#[test]
fn pathcch_skip_root() {
    run("PathCchSkipRoot", include_str!("data/PathCchSkipRoot.txt"), |l, r| {
        let p = w(quoted(l)[0]);
        match wp::pathcch_skip_root(&p) {
            None => eq(r.trim().to_string(), "0x80070057".to_string()),
            Some(at) => {
                if r.trim() == "0x80070057" {
                    return Some(format!("got a root end at {at}, want E_INVALIDARG"));
                }
                let want = unquoted(r);
                eq((format!("0x0, rest at +{at} "), s(&p[at..])), (want, quoted(r)[0].to_string()))
            }
        }
    });
}

#[test]
fn remove_file_spec() {
    run("PathRemoveFileSpecW", include_str!("data/PathRemoveFileSpecW.txt"), |l, r| {
        let p = w(quoted(l)[0]);
        let (out, changed) = wp::remove_file_spec(&p);
        eq((s(&out), i64::from(changed)), (quoted(r)[0].to_string(), num(r.rsplit(" ret ").next().unwrap())))
    });
}

#[test]
fn strip_to_root() {
    run("PathStripToRootW", include_str!("data/PathStripToRootW.txt"), |l, r| {
        let p = w(quoted(l)[0]);
        let (out, has) = wp::strip_to_root(&p);
        eq((s(&out), i64::from(has)), (quoted(r)[0].to_string(), num(r.rsplit(" ret ").next().unwrap())))
    });
}

#[test]
fn add_backslash() {
    run("PathAddBackslashW", include_str!("data/PathAddBackslashW.txt"), |l, r| {
        let mut p = w(quoted(l)[0]);
        wp::add_backslash(&mut p);
        // `+N "" string now "S"`.
        eq((format!("+{}", p.len()), s(&p)), (r.split(' ').next().unwrap().to_string(), quoted(r)[1].to_string()))
    });
}

#[test]
fn remove_backslash() {
    run("PathRemoveBackslashW", include_str!("data/PathRemoveBackslashW.txt"), |l, r| {
        let mut p = w(quoted(l)[0]);
        let removed = wp::remove_backslash(&mut p);
        // The result points at the removed backslash's place, or at the last character.
        let at = if removed { p.len() } else { p.len().saturating_sub(1) };
        eq(
            (format!("+{at}"), s(&p[at..]), s(&p)),
            (r.split(' ').next().unwrap().to_string(), quoted(r)[0].to_string(), quoted(r)[1].to_string()),
        )
    });
}

#[test]
fn path_canonicalize() {
    run("PathCanonicalizeW", include_str!("data/PathCanonicalizeW.txt"), |l, r| {
        let out = wp::path_canonicalize(&w(quoted(l)[0])).expect("fits");
        eq(s(&out), quoted(r)[0].to_string())
    });
}

#[test]
fn path_combine() {
    run("PathCombineW", include_str!("data/PathCombineW.txt"), |l, r| {
        let q = quoted(l);
        let got = wp::path_combine(&w(q[0]), &w(q[1]));
        let (null, out) = match got {
            Ok(v) => (false, s(&v)),
            Err(_) => (true, String::new()),
        };
        eq((null, out), (r.starts_with("NULL"), quoted(r)[0].to_string()))
    });
}

#[test]
fn path_append() {
    run("PathAppendW", include_str!("data/PathAppendW.txt"), |l, r| {
        let q = quoted(l);
        let (out, ret) = match wp::path_append(&w(q[0]), &w(q[1])) {
            Ok(v) => (s(&v), 1),
            Err(_) => (String::new(), 0),
        };
        eq((out, ret), (quoted(r)[0].to_string(), num(r.rsplit(" ret ").next().unwrap())))
    });
}

/// `cch N flags 0xF` of a PathCch line: (cch, flags).
fn cch_flags(l: &str) -> (usize, u32) {
    let t = unquoted(l);
    let mut it = t.split_whitespace();
    assert_eq!(it.next(), Some("cch"));
    let cch = it.next().unwrap().parse().unwrap();
    assert_eq!(it.next(), Some("flags"));
    let flags = u32::from_str_radix(it.next().unwrap().trim_start_matches("0x"), 16).unwrap();
    (cch, flags)
}

#[test]
fn pathcch_canonicalize_ex() {
    run("PathCchCanonicalizeEx", include_str!("data/PathCchCanonicalizeEx.txt"), |l, r| {
        let (cch, flags) = cch_flags(l);
        let src = w(quoted(l)[0]);
        let text = wp::pathcch_canonical_text(&src, flags);
        let (hr, out) = match wp::pathcch_fit(text, cch, flags) {
            Ok(v) => (0, s(&v)),
            Err(hr) => (hr, String::new()),
        };
        eq((hr, out), (num(r.split(',').next().unwrap()) as u32, quoted(r)[0].to_string()))
    });
}

#[test]
fn pathcch_combine_ex() {
    run("PathCchCombineEx", include_str!("data/PathCchCombineEx.txt"), |l, r| {
        let (cch, flags) = cch_flags(l);
        let q = quoted(l);
        let (hr, out) = match wp::pathcch_combine_text(&w(q[0]), &w(q[1]), flags) {
            None => (wp::E_INVALIDARG, String::new()),
            Some(t) => match wp::pathcch_fit(t, cch, flags) {
                Ok(v) => (0, s(&v)),
                Err(hr) => (hr, String::new()),
            },
        };
        eq((hr, out), (num(r.split(',').next().unwrap()) as u32, quoted(r)[0].to_string()))
    });
}
