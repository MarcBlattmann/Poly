//! `/bin/native-hello`: the first native Poly program (ADR 0125), the
//! native boot test's first process. It prints, reads its arguments and
//! environment, allocates, keeps time, writes and reads a file, spawns
//! native children (one exits with a status, one panics, one faults) and,
//! when it is there, the Linux hello world `/bin/hello` on the same
//! console handle, and checks each. It ends with
//! `native-hello: done, <n> failed` and exit status 0 if nothing failed.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use poly_rt::abi::ExitStatus;
use poly_rt::process::Command;
use poly_rt::time::Instant;
use poly_rt::{Error, env, fs, println, process, time};

poly_rt::entry!(main);

/// Count a failed check and say which.
fn check(failed: &mut u32, what: &str, ok: bool) {
    if !ok {
        println!("native-hello: FAIL {what}");
        *failed += 1;
    }
}

fn main() -> i32 {
    let mut failed = 0;
    println!("native-hello: hello from Poly's native user space (pid {})", process::id());

    let args: Vec<&str> = env::args_str().collect();
    println!("native-hello: arguments {args:?}");
    check(&mut failed, "argv[0] names the program", args.first().is_some_and(|a| a.ends_with("native-hello")));

    // The heap: many small objects (slabs), one large run, reuse.
    let numbers: Vec<u64> = (0..100_000).collect();
    check(&mut failed, "a vector of 100000 numbers", numbers.iter().sum::<u64>() == 4_999_950_000);
    let big = alloc::vec![0xa5u8; 3 << 20];
    check(&mut failed, "a 3 MiB allocation", big.len() == 3 << 20 && big.iter().all(|&b| b == 0xa5));
    drop(big);
    let mut map = BTreeMap::new();
    for i in 0..2000u32 {
        map.insert(format!("key-{i:04}"), String::from("value").repeat(1 + i as usize % 7));
    }
    for round in 0..20 {
        let strings: Vec<String> = (0..500).map(|i| format!("{round}:{i}")).collect();
        check(&mut failed, "freed memory is reused", strings.len() == 500);
    }
    check(&mut failed, "a map of 2000 strings", map.len() == 2000 && map["key-1999"].len() == 5 * 5);
    println!("native-hello: heap ok ({} map entries)", map.len());

    // Time.
    let start = Instant::now();
    check(&mut failed, "sleep", time::sleep(Duration::from_millis(20)).is_ok());
    let slept = start.elapsed();
    check(&mut failed, "the monotonic clock moved by the sleep", slept >= Duration::from_millis(20));
    check(&mut failed, "the wall clock is past 2023", time::since_epoch().as_secs() > 1_700_000_000);
    println!("native-hello: slept {} ms", slept.as_millis());

    // Files, through the same VFS Linux programs use.
    let text = b"written by a native program\n";
    check(&mut failed, "writing a file", fs::write("/native-hello.txt", text).is_ok());
    check(&mut failed, "reading it back", fs::read("/native-hello.txt").as_deref() == Ok(&text[..]));
    check(&mut failed, "a missing file", fs::read("/no/such/file").err() == Some(Error::NotFound));
    let motd = fs::read("/etc/hostname").unwrap_or_default();
    println!("native-hello: /etc/hostname is {:?}", core::str::from_utf8(&motd).unwrap_or("?").trim_end());

    // Children: a native one's exit status, a panic, a fault.
    let status = Command::new("/bin/native-child").args(["exit", "42"]).status();
    println!("native-hello: native-child exit 42 ended: {status:?}");
    check(&mut failed, "a native child's exit status", status == Ok(ExitStatus::Exited(42)));
    let status = Command::new("/bin/native-child").arg("panic").status();
    check(&mut failed, "a native child's panic exits 101", status == Ok(ExitStatus::Exited(101)));
    let status = Command::new("/bin/native-child").arg("fault").status();
    println!("native-hello: native-child fault ended: {status:?}");
    check(&mut failed, "a native child's page fault", status == Ok(ExitStatus::Faulted(14)));
    let missing = Command::new("/bin/no-such-program").status();
    check(&mut failed, "spawning a missing program", missing == Err(Error::NotFound));

    // A Linux program started by a native one, sharing its console handle.
    if fs::File::open("/bin/hello").is_ok() {
        let status = Command::new("/bin/hello").status();
        println!("native-hello: the Linux /bin/hello ended: {status:?}");
        check(&mut failed, "a Linux child", status == Ok(ExitStatus::Exited(0)));
    }

    println!("native-hello: done, {failed} failed");
    if failed == 0 { 0 } else { 1 }
}
