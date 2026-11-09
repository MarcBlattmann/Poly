//! Tests.

use super::*;

#[test]
fn shards_partition_the_names() {
    let names = ["d", "a", "e", "c", "b", "f", "g"];
    let mut all: Vec<String> = (1..=3).flat_map(|i| shard_slice(&names, i, 3)).collect();
    assert_eq!(shard_slice(&names, 1, 3), ["a", "d", "g"]);
    assert_eq!(shard_slice(&names, 2, 3), ["b", "e"]);
    assert_eq!(shard_slice(&names, 3, 3), ["c", "f"]);
    all.sort();
    assert_eq!(all, ["a", "b", "c", "d", "e", "f", "g"]);
    assert_eq!(shard_slice(&names, 4, 5), ["d"]);
    assert!(shard_slice(&["a"], 2, 3).is_empty());
}

#[test]
fn parses_a_shard() {
    assert_eq!(parse_shard("2/3"), Ok((2, 3)));
    assert_eq!(parse_shard("1/1"), Ok((1, 1)));
    for bad in ["0/3", "4/3", "2", "a/b", "2/", "/3", "2/0"] {
        assert!(parse_shard(bad).is_err(), "{bad}");
    }
}

/// The real tracer on the real hello world, where a C compiler exists.
#[test]
fn traces_hello_world() {
    let cc = corpus::compiler(None);
    if corpus::compiler_version(&cc).is_err() {
        eprintln!("skipped: no C compiler");
        return;
    }
    let dir = std::env::temp_dir().join(format!("poly-pipeline-hello-{}", std::process::id()));
    let m = corpus::build(&repo_root(), &dir, &cc).unwrap();
    match ptrace::trace(&dir.join("hello"), "hello", DEFAULT_TIMEOUT) {
        Err(e) if e.contains("not permitted") => eprintln!("skipped: {e}"),
        r => check_hello(&r.unwrap()).unwrap(),
    }
    // A program killed by a signal, and one that fails its checks never
    // happens in the corpus: everything passes on Linux.
    let segv = ptrace::trace(&dir.join("segv"), "segv", DEFAULT_TIMEOUT);
    if let Ok(t) = segv {
        assert_eq!(t.exit, trace::Exit::Signal(11));
        assert!(t.events.iter().any(|e| matches!(e, trace::Event::Signal { signal: 11, .. })));
    }
    let untraced = runner::run(Tracer::None, &dir.join("exit42"), "exit42", DEFAULT_TIMEOUT).unwrap();
    assert_eq!(untraced.exit, trace::Exit::Code(42));
    assert!(m.tests.len() >= 11);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A child that leaves the test's process group (setsid) is still
/// traced to its end, and one that hangs there is killed on timeout.
#[test]
fn traces_children_that_leave_the_process_group() {
    let cc = corpus::compiler(None);
    if corpus::compiler_version(&cc).is_err() {
        eprintln!("skipped: no C compiler");
        return;
    }
    let base = std::env::temp_dir().join(format!("poly-pipeline-setsid-{}", std::process::id()));
    let src = base.join("src");
    util::create_dir(&src).unwrap();
    // fork (57); in the child setsid (112), getppid (110), exit_group 3;
    // the parent waits with wait4 (61) and exits 0. With argument
    // "hang" the child spins after setsid instead.
    let program = r#"#include "poly_test.h"
int test_main(void) {
i64 pid = syscall6(57, 0, 0, 0, 0, 0, 0);
if (pid == 0) {
    syscall6(112, 0, 0, 0, 0, 0, 0);
    if (poly_initial_sp[0] > 1) for (;;) {}
    syscall6(110, 0, 0, 0, 0, 0, 0);
    exit_group(3);
}
int status = 0;
syscall6(61, pid, (i64)&status, 0, 0, 0, 0);
return ((status >> 8) & 0xff) == 3 ? 0 : 1;
}
"#;
    util::write(&src.join("leaves_group.c"), program).unwrap();
    let bin = base.join("bin");
    corpus::build_with(&repo_root(), &bin, &cc, &[src]).unwrap();
    let t = match ptrace::trace(&bin.join("leaves_group"), "leaves_group", DEFAULT_TIMEOUT) {
        Err(e) if e.contains("not permitted") => return eprintln!("skipped: {e}"),
        r => r.unwrap(),
    };
    assert_eq!(t.exit, trace::Exit::Code(0));
    let names: Vec<(u32, &str)> = t
        .events
        .iter()
        .filter_map(|e| match e {
            trace::Event::Syscall(c) => Some((c.pid, c.name.as_str())),
            _ => None,
        })
        .collect();
    assert!(names.contains(&(1, "setsid")) && names.contains(&(1, "getppid")), "{names:?}");
    assert!(names.contains(&(1, "exit_group")) && names.contains(&(0, "wait4")), "{names:?}");

    // The hanging variant: argv[1] given through the trace name is not
    // possible, so run the same binary through a wrapper argument.
    let hang = base.join("hang.sh");
    util::write(&hang, format!("#!/bin/sh\nexec {} hang\n", bin.join("leaves_group").display())).unwrap();
    std::fs::set_permissions(&hang, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let start = std::time::Instant::now();
    let t = ptrace::trace(&hang, "hang", Duration::from_secs(2)).unwrap();
    assert_eq!(t.exit, trace::Exit::Timeout);
    assert!(start.elapsed() < Duration::from_secs(30), "the watchdog reached the session child");
    let _ = std::fs::remove_dir_all(&base);
}
