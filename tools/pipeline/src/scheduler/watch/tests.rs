//! The dashboard against a fabricated state directory, log and `/proc`.

use super::gather::{self, Limits, Sources};
use super::render;
use crate::scheduler::state::{self, EntryState, Event, Pause, Step, Store};
use crate::util;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// 2026-10-20 12:00:00 UTC.
const NOW: u64 = 1_792_454_400 + 12 * 3600;
const BOOT: u64 = NOW - 100_000;

const BRK: &str = "linux/syscall/x86_64/brk";
const MMAP: &str = "linux/syscall/x86_64/mmap";
const OPENAT: &str = "linux/syscall/x86_64/openat";
const CHDIR: &str = "linux/syscall/x86_64/chdir";
const GETPID: &str = "linux/syscall/x86_64/getpid";
const OLD: &str = "linux/syscall/x86_64/getuid";

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("poly-watch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    util::create_dir(&d).unwrap();
    d
}

fn t(secs: u64) -> String {
    util::format_utc(secs)
}

/// An entry that went through `steps` (each entered at the given time)
/// and is now at the last one.
fn entry(id: &str, steps: &[(Step, u64)]) -> EntryState {
    let mut s = EntryState::new(id, steps[0].1);
    for w in steps.windows(2) {
        // The note that leaves a step is written under it.
        s.history.push(Event { time: t(w[1].1), step: w[0].0, message: format!("{} done", w[0].0.name()) });
    }
    s.step = steps.last().unwrap().0;
    s.updated = t(steps.last().unwrap().1);
    s
}

fn limits() -> Limits {
    Limits {
        max_parallel: 2,
        max_cycles: 5,
        max_errors: 8,
        spec_cycles: 2,
        poll_secs: 600,
        id_prefix: "linux/syscall/".into(),
    }
}

fn process(proc_dir: &Path, pid: u32, ppid: u32, uid: u32, started: u64, argv: &[&str]) {
    let d = proc_dir.join(pid.to_string());
    let filler = vec!["0"; 17].join(" ");
    let ticks = (started - BOOT) * 100;
    util::write(&d.join("stat"), format!("{pid} (a (b) c) S {ppid} {filler} {ticks} 0 0\n")).unwrap();
    util::write(&d.join("cmdline"), argv.join("\0") + "\0").unwrap();
    util::write(&d.join("status"), format!("Name:\tx\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n")).unwrap();
}

struct Fixture {
    dir: PathBuf,
    src: Sources,
}

fn fixture(name: &str) -> Fixture {
    let dir = temp(name);
    let state_dir = dir.join("state");
    let store = Store::new(&state_dir).unwrap();

    // brk: implementer session running for 20 minutes, cycle 2.
    let mut brk = entry(
        BRK,
        &[
            (Step::Claim, NOW - 7200),
            (Step::Spec, NOW - 7100),
            (Step::Handoff, NOW - 3000),
            (Step::Implement, NOW - 1200),
        ],
    );
    brk.cycle = 2;
    store.save(&brk).unwrap();
    // mmap: CI in its verify step, one error before.
    let mut mmap = entry(MMAP, &[(Step::Claim, NOW - 5000), (Step::Verify, NOW - 300)]);
    mmap.errors = 1;
    store.save(&mmap).unwrap();
    // openat: PR waiting for its merge.
    let mut openat = entry(OPENAT, &[(Step::Claim, NOW - 90_000), (Step::AwaitingMerge, NOW - 3600)]);
    openat.pr_url = Some("https://github.com/o/poly/pull/12".into());
    openat.last_poll = NOW - 100;
    store.save(&openat).unwrap();
    // chdir: in the human queue (its note is written under `human`).
    let mut chdir = entry(CHDIR, &[(Step::Claim, NOW - 20_000), (Step::Verify, NOW - 9000)]);
    chdir.step = Step::Human;
    chdir.cycle = 5;
    chdir.reason = Some("5 red cycles; last:\n2 of 6 held-out tests fail".into());
    chdir.history.push(Event { time: t(NOW - 4000), step: Step::Human, message: "to the human queue".into() });
    store.save(&chdir).unwrap();
    // getpid: merged an hour ago; getuid: merged two days ago (counted, not listed).
    let mut getpid =
        entry(GETPID, &[(Step::Claim, NOW - 30_000), (Step::AwaitingMerge, NOW - 7200), (Step::Merged, NOW - 3600)]);
    getpid.pr_url = Some("https://github.com/o/poly/pull/9".into());
    store.save(&getpid).unwrap();
    store.save(&entry(OLD, &[(Step::Claim, NOW - 300_000), (Step::Merged, NOW - 200_000)])).unwrap();
    // A write in progress (the loop renames it into place) and a broken file.
    util::write(&state_dir.join("entries/linux__syscall__x86_64__brk.tmp.4242"), "{\"format\": \"poly-lo").unwrap();
    util::write(&state_dir.join("entries/linux__syscall__x86_64__bad.json"), "{ not json").unwrap();

    store.set_pause("implementer", &Pause { until: NOW + 3540, reason: "usage limit reached".into() }).unwrap();
    store.set_pause("spec", &Pause { until: NOW - 60, reason: "old".into() }).unwrap();

    // Command logs, named as RealOps names them.
    let stamp = |secs: u64| util::format_utc(secs).replace(['-', ':'], "").replace(' ', "-");
    let logs = |id: &str| state_dir.join("logs").join(state::key(id));
    util::write(&logs(BRK).join(format!("{}-100-001-implementer-sync.log", stamp(NOW - 1195))), "$ sudo\n").unwrap();
    util::write(&logs(BRK).join(format!("{}-100-002-implementer.log", stamp(NOW - 1190))), "$ sudo\n").unwrap();
    util::write(&logs(MMAP).join(format!("{}-100-001-handoff-verify.log", stamp(NOW - 290))), "$ cargo\n").unwrap();
    util::write(&logs(MMAP).join(format!("{}-100-002-ci.log", stamp(NOW - 250))), "$ cargo xtask ci\n").unwrap();

    // The lock files the loop holds: the two entries and the implementer role.
    for name in ["entry-linux__syscall__x86_64__brk", "entry-linux__syscall__x86_64__mmap", "role-implementer", "pick"]
    {
        util::write(&state_dir.join(format!("locks/{name}.lock")), "").unwrap();
    }

    let log_file = dir.join("log/pipeline.log");
    let mut log = String::new();
    for i in 0..20 {
        log.push_str(&format!("{} w1 {OPENAT}: [awaiting-merge] poll {i}\n", t(NOW - 4000 + i)));
    }
    log.push_str(&format!("{} w0 {BRK}: [implement] implementer session starts\n", t(NOW - 1190)));
    log.push_str(&format!("{} w1 {MMAP}: [verify] handed off; CI\n", t(NOW - 250)));
    util::write(&log_file, log).unwrap();

    let proc_dir = dir.join("proc");
    util::write(&proc_dir.join("stat"), format!("cpu 1 2 3\nbtime {BOOT}\n")).unwrap();
    util::write(&proc_dir.join("loadavg"), "2.31 1.80 1.52 3/412 9999\n").unwrap();
    util::write(&proc_dir.join("meminfo"), "MemTotal:       16384000 kB\nMemFree: 1 kB\nMemAvailable:    8192000 kB\n")
        .unwrap();
    let passwd = dir.join("passwd");
    util::write(&passwd, "root:x:0:0::/root:/bin/sh\nserver:x:1000:1000::/home/server:/bin/bash\npolyimpl:x:998:998::/srv/poly/impl:/usr/sbin/nologin\n").unwrap();
    let runner = "/srv/poly/runner/repo/target/debug/pipeline";
    process(&proc_dir, 90, 1, 1000, NOW - 90_000, &["cargo", "xtask", "pipeline", "loop"]);
    process(&proc_dir, 100, 90, 1000, NOW - 86_000, &[runner, "loop"]);
    process(&proc_dir, 101, 50, 1000, NOW - 10, &[runner, "loop", "status"]);
    process(&proc_dir, 102, 50, 1000, NOW - 10, &[runner, "loop", "watch"]);
    // brk's session: sudo (root), the wrapper, cargo, then Claude Code as polyimpl.
    process(
        &proc_dir,
        200,
        100,
        0,
        NOW - 1190,
        &["sudo", "-n", "-u", "polyimpl", "/srv/poly/bin/poly-role", "implementer", "run", BRK, "impl-agent"],
    );
    process(&proc_dir, 201, 200, 998, NOW - 1189, &["/bin/sh", "/srv/poly/bin/poly-role", "implementer", "run", BRK]);
    process(&proc_dir, 202, 201, 998, NOW - 1188, &["cargo", "xtask", "pipeline", "role", "implementer", BRK]);
    process(
        &proc_dir,
        203,
        202,
        998,
        NOW - 1180,
        &["/srv/poly/impl/.local/bin/claude", "-p", "--output-format", "json"],
    );
    // mmap's CI: no entry among its arguments; its log names it.
    process(&proc_dir, 210, 100, 1000, NOW - 249, &["cargo", "xtask", "ci"]);
    // A kernel thread: no command line.
    util::write(&proc_dir.join("2/stat"), "2 (kthreadd) S 0 0").unwrap();
    util::write(&proc_dir.join("2/cmdline"), "").unwrap();

    let src = Sources {
        state_dir,
        log_file: Some(log_file),
        proc_dir,
        passwd,
        disk_path: dir.clone(),
        service: None,
        host: "srv".into(),
        config: Some("/srv/poly/runner/pipeline.toml".into()),
        clock_ticks: 100,
        log_lines: 8,
    };
    Fixture { dir, src }
}

/// `/proc/locks` naming the lock files: `held` by the loop (pid 100) on
/// their device, `other` held by pid 7 on another device.
fn proc_locks(f: &Fixture, held: &[&str], other: &[&str]) {
    let mut text = String::new();
    let mut n = 1;
    for (names, pid, real_dev) in [(held, 100, true), (other, 7, false)] {
        for name in names {
            let m = std::fs::metadata(f.src.state_dir.join(format!("locks/{name}.lock"))).unwrap();
            let (maj, min) = if real_dev { gather::dev_numbers(m.dev()) } else { (0xfe, 0x7) };
            text.push_str(&format!("{n}: FLOCK  ADVISORY  WRITE {pid} {maj:02x}:{min:02x}:{} 0 EOF\n", m.ino()));
            text.push_str(&format!("{n}: -> FLOCK  ADVISORY  WRITE 9 {maj:02x}:{min:02x}:{} 0 EOF\n", m.ino()));
            n += 1;
        }
    }
    util::write(&f.src.proc_dir.join("locks"), text).unwrap();
}

const HELD: &[&str] = &["entry-linux__syscall__x86_64__brk", "entry-linux__syscall__x86_64__mmap", "role-implementer"];

#[test]
fn gathers_entries_activity_and_host_from_files_and_proc() {
    let f = fixture("gather");
    proc_locks(&f, HELD, &["pick"]);
    let s = gather::gather(&f.src, &limits(), NOW);

    // Rows: running first, then waiting, then the human queue and done; old merges only counted.
    let order: Vec<(&str, &str, bool)> =
        s.entries.iter().map(|r| (r.entry.as_str(), r.stage.as_str(), r.running)).collect();
    assert_eq!(
        order,
        [
            (BRK, "implement", true),
            (MMAP, "verify", true),
            (OPENAT, "waiting", false),
            (CHDIR, "human", false),
            (GETPID, "done", false)
        ]
    );
    let row = |id: &str| s.entries.iter().find(|r| r.entry == id).unwrap();
    assert_eq!(row(BRK).since, Some(NOW - 1200));
    assert_eq!(row(BRK).cycle, 2);
    assert_eq!(row(BRK).worker.as_deref(), Some("w0"));
    assert_eq!(row(MMAP).worker.as_deref(), Some("w1"));
    assert_eq!(row(CHDIR).since, Some(NOW - 4000), "the human queue's own note");
    assert!(row(CHDIR).reason.as_deref().unwrap().starts_with("5 red cycles"));
    let w = row(OPENAT).wait.as_ref().unwrap();
    assert_eq!((w.what.as_str(), w.until), ("next PR check", NOW + 500));
    assert_eq!(s.totals, gather::Totals { done_today: 1, done: 2, human: 1, open_prs: 1, in_flight: 2 });

    assert_eq!(s.problems.len(), 1, "{:?}", s.problems);
    assert!(s.problems[0].contains("bad.json"));

    assert_eq!(s.pauses.len(), 2);
    let p = s.pauses.iter().find(|p| p.active).unwrap();
    assert_eq!((p.role.as_str(), p.until), ("implementer", NOW + 3540));
    assert_eq!(s.roles_busy, ["implementer"]);

    assert_eq!(s.loops.iter().map(|p| p.pid).collect::<Vec<_>>(), [100]);
    assert_eq!(s.loops[0].started, NOW - 86_000);
    assert_eq!(s.load, Some([2.31, 1.80, 1.52]));
    assert_eq!(s.memory, Some(gather::Space { free: 8_192_000 * 1024, total: 16_384_000 * 1024 }));
    assert!(s.disk.as_ref().is_some_and(|d| d.space.total > 0));

    assert_eq!(s.activity.len(), 2, "{:#?}", s.activity);
    let a = &s.activity[0];
    assert_eq!(
        (a.worker.as_deref(), a.entry.as_deref(), a.what.as_str()),
        (Some("w0"), Some(BRK), "implementer session")
    );
    assert_eq!((a.pid, a.started, a.exact), (Some(200), Some(NOW - 1190), true));
    let agent = a.agent.as_ref().unwrap();
    assert_eq!((agent.pid, agent.user.as_deref(), agent.program.as_str()), (203, Some("polyimpl"), "claude"));
    assert!(a.log.as_deref().unwrap().ends_with("-100-002-implementer.log"));
    let ci = &s.activity[1];
    assert_eq!((ci.entry.as_deref(), ci.what.as_str(), ci.pid), (Some(MMAP), "CI", Some(210)));
    assert!(ci.log.as_deref().unwrap().ends_with("-ci.log"));

    assert_eq!(s.log_tail.len(), 8);
    assert!(s.log_tail.last().unwrap().contains("[verify] handed off; CI"));
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn without_the_loop_in_the_process_list_activity_comes_from_the_logs() {
    let f = fixture("nolist");
    proc_locks(&f, HELD, &[]);
    for pid in [90, 100, 200, 201, 202, 203, 210] {
        std::fs::remove_dir_all(f.src.proc_dir.join(pid.to_string())).unwrap();
    }
    // Held on their own device: found without knowing the holder.
    let s = gather::gather(&f.src, &limits(), NOW);
    assert!(s.loops.is_empty());
    let acts: Vec<(&str, &str, Option<u64>, bool)> =
        s.activity.iter().map(|a| (a.entry.as_deref().unwrap(), a.what.as_str(), a.started, a.exact)).collect();
    assert_eq!(acts, [(BRK, "implementer session", Some(NOW - 1190), false), (MMAP, "CI", Some(NOW - 250), false)]);
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn locks_on_other_devices_and_pids_do_not_count_and_probing_works_without_proc_locks() {
    let f = fixture("locks");
    proc_locks(&f, &[], HELD);
    let s = gather::gather(&f.src, &limits(), NOW);
    assert!(s.entries.iter().all(|r| !r.running));
    assert!(s.roles_busy.is_empty());
    // Not one between commands either: nothing runs, the CI child is unrelated to a running entry.
    assert!(s.activity.iter().all(|a| !a.what.contains("between")));

    // No /proc/locks: a held lock is seen by probing, a free one is left as it was.
    std::fs::remove_file(f.src.proc_dir.join("locks")).unwrap();
    let store = Store { dir: f.src.state_dir.clone() };
    let held = store.try_lock("entry-linux__syscall__x86_64__mmap").unwrap().unwrap();
    let s = gather::gather(&f.src, &limits(), NOW);
    let running: Vec<&str> = s.entries.iter().filter(|r| r.running).map(|r| r.entry.as_str()).collect();
    assert_eq!(running, [MMAP]);
    drop(held);
    assert!(store.try_lock("entry-linux__syscall__x86_64__brk").unwrap().is_some(), "the probe let go");
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn an_empty_or_missing_state_dir_is_not_an_error() {
    let dir = temp("empty");
    let src = Sources {
        state_dir: dir.join("nope"),
        log_file: Some(dir.join("nope.log")),
        proc_dir: dir.join("noproc"),
        passwd: dir.join("nopasswd"),
        disk_path: dir.join("nope/deeper"),
        service: None,
        host: "h".into(),
        config: None,
        clock_ticks: 100,
        log_lines: 8,
    };
    let s = gather::gather(&src, &limits(), NOW);
    assert!(s.entries.is_empty() && s.activity.is_empty() && s.problems.is_empty() && s.log_tail.is_empty());
    assert!(s.disk.is_some(), "the nearest existing directory's file system");
    let text: Vec<String> = render::render(&s, None).iter().map(|l| render::to_text(l, 80, false)).collect();
    assert!(text.iter().any(|l| l.contains("no entries yet")));
    assert!(text.iter().any(|l| l.contains("no loop process seen")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renders_within_80_columns_and_the_terminal_height() {
    let f = fixture("render");
    proc_locks(&f, HELD, &[]);
    let s = gather::gather(&f.src, &limits(), NOW);
    let plain: Vec<String> = render::render(&s, None).iter().map(|l| render::to_text(l, 80, false)).collect();
    let all = plain.join("\n");
    println!("{all}");
    assert!(plain.iter().all(|l| l.chars().count() <= 80 && l.is_ascii()), "{all}");
    for want in [
        "service unknown  loop pid 100 up 23h53m  max_parallel 2",
        "PAUSED implementer until 12:59 UTC (in 59m00s): usage limit reached",
        "done today 1 (2 in all)  human queue 1  open PRs 1  in flight 2/2",
        // Every id starts with linux/syscall/x86_64/: said once, above the table.
        "ENTRIES (linux/syscall/x86_64/...)\n  ENTRY  STEP       CYCLE ERR  IN STEP NOTE\n",
        "* brk    implement  2/5   0/8  20m00s  running in w0",
        "* mmap   verify     0/5   1/8  5m00s   running in w1",
        "  openat waiting    0/5   0/8  1h00m   github.com/o/poly/pull/12, check in 8m20s",
        "  chdir  human      5/5   0/8  1h06m   5 red cycles; last: 2 of 6 held-out tests",
        "  getpid done       0/5   0/8  -       github.com/o/poly/pull/9",
        "  w0  brk    implementer session 19m50s  pid 200\n             claude pid 203 as polyimpl up 19m40s; log written ",
        "  w1  mmap   CI 4m09s  pid 210",
        "11:55:50 w1 mmap: [verify] handed off; CI",
    ] {
        assert!(all.contains(want), "missing {want:?} in\n{all}");
    }
    // The old merge is not listed.
    assert!(!all.contains("getuid"));

    // The shared prefix: whole parts, never a symbol, at least <os>/<layer>/.
    let mut mixed = s.clone();
    assert_eq!(render::shared_prefix(&mixed), "linux/syscall/x86_64/");
    mixed.entries[0].entry = "linux/syscall/common/brk".into();
    assert_eq!(render::shared_prefix(&mixed), "linux/syscall/");
    mixed.entries[0].entry = "nt/syscall/NtClose".into();
    assert_eq!(render::shared_prefix(&mixed), "linux/syscall/");
    mixed.entries.truncate(1);
    mixed.activity.clear();
    assert_eq!(render::shared_prefix(&mixed), "nt/syscall/");

    // Coloured: escape codes, but the same text.
    let colored = render::to_text(&render::render(&s, None)[0], 80, true);
    assert!(colored.contains("\x1b[1mPOLY PIPELINE\x1b[0m"));

    // A short terminal: the table gives way, the log and activity stay.
    let short = render::render(&s, Some(24));
    assert!(short.len() <= 24);
    let text: Vec<String> = short.iter().map(|l| render::to_text(l, 80, false)).collect();
    assert!(text.iter().any(|l| l.contains("more (loop status lists all)")), "{}", text.join("\n"));
    assert!(text.iter().any(|l| l.starts_with("ACTIVITY")));

    // --json: the whole snapshot.
    let json: serde_json::Value = serde_json::to_value(&s).unwrap();
    assert_eq!(json["totals"]["human"], 1);
    assert_eq!(json["entries"][0]["stage"], "implement");
    assert_eq!(json["activity"][0]["agent"]["user"], "polyimpl");
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn shows_pauses_without_an_end_and_the_loops_own_notices() {
    let f = fixture("notices");
    let store = Store { dir: f.src.state_dir.clone() };
    store.set_pause("spec", &Pause { until: state::FOREVER, reason: "loop pause by the owner".into() }).unwrap();
    let failure = crate::scheduler::update::BuildFailure {
        head: "abc".into(),
        time: t(NOW - 60),
        message: "the new version at abc does not build".into(),
    };
    util::write_json(&crate::scheduler::update::failure_file(&f.src.state_dir), &failure).unwrap();
    let s = gather::gather(&f.src, &limits(), NOW);
    let all: Vec<String> = render::render(&s, None).iter().map(|l| render::to_text(l, 100, false)).collect();
    let all = all.join("\n");
    assert!(all.contains("PAUSED spec until `loop resume` (no end): loop pause by the owner"), "{all}");
    assert!(
        all.contains("NOTICE self-update: the new version at abc does not build (since 2026-10-20 11:59:00 UTC)"),
        "{all}"
    );
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn parses_proc_files_log_names_and_commands() {
    assert_eq!(gather::parse_stat("42 (a (b) c) S 7 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1234 9"), Some((7, 1234)));
    assert_eq!(gather::parse_stat("garbage"), None);
    let locks = gather::parse_proc_locks(
        "1: FLOCK  ADVISORY  WRITE 91 fe:00:663556 0 EOF\n2: -> FLOCK  ADVISORY  WRITE 3 fe:00:1 0 EOF\n\
         3: POSIX  ADVISORY  WRITE 5 08:01:9 0 EOF\n",
    );
    assert_eq!(locks, [gather::HeldLock { pid: 91, major: 0xfe, minor: 0, inode: 663_556 }]);
    assert_eq!(
        gather::parse_log_name("20261020-115000-100-002-handoff-verify.log"),
        Some((NOW - 600, 100, 2, "handoff-verify".into()))
    );
    assert_eq!(gather::parse_log_name("20261020-115000-100-002-pr-body.md"), None);

    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        gather::label(&v(&["sudo", "-n", "-u", "polyspec", "/srv/poly/bin/poly-role", "spec", "run", BRK])),
        "spec agent session"
    );
    assert_eq!(
        gather::label(&v(&["sudo", "-u", "polyimpl", "/srv/poly/bin/poly-role", "implementer", "sync", "b", "m"])),
        "implementer checkout sync"
    );
    assert_eq!(
        gather::label(&v(&["/home/server/.cargo/bin/cargo", "xtask", "pipeline", "diff", "--target", "poly"])),
        "diff run"
    );
    assert_eq!(gather::label(&v(&["cargo", "xtask", "pipeline", "handoff", "verify", BRK])), "pipeline handoff verify");
    assert_eq!(gather::label(&v(&["git", "-C", "/r", "-c", "user.name=x", "fetch", "-q", "hub"])), "git fetch");
    assert_eq!(gather::label(&v(&["gh", "pr", "view", "12"])), "gh pr view");
    assert!(gather::is_loop(&v(&["target/debug/pipeline", "loop", "--once", "--entry", BRK])));
    assert!(!gather::is_loop(&v(&["target/debug/pipeline", "loop", "--dry-run"])));
    assert!(!gather::is_loop(&v(&["cargo", "xtask", "pipeline", "loop"])));
    assert!(gather::is_agent(&v(&["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"])));
    assert!(!gather::is_agent(&v(&["cargo", "xtask", "pipeline", "role", "spec", "--preset", "claude"])));

    assert_eq!(render::duration(42), "42s");
    assert_eq!(render::duration(725), "12m05s");
    assert_eq!(render::duration(3 * 3600 + 240), "3h04m");
    assert_eq!(render::duration(2 * 86_400 + 5 * 3600), "2d05h");
    assert_eq!(render::bytes(8_192_000 * 1024), "7.8G");
    assert_eq!(render::bytes(512), "512B");
    assert_eq!(render::clock(NOW + 60, NOW), "12:01 UTC");
    assert_eq!(render::clock(NOW + 86_400, NOW), "10-21 12:00 UTC");
}
