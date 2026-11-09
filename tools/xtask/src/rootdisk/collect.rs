//! Collecting what the image holds, the gcc parts included.

use super::*;

/// Collect what the image holds (see the module documentation).
pub fn collect() -> Result<Contents, String> {
    collect_with(|_, _, _| Ok(()))
}

/// [`collect`] with more from `extra` (the toolkits' disk, `toolkits.rs`):
/// it adds host files to the collector (their shared objects are added
/// after it returns), the programs it found and notes.
pub(crate) fn collect_with(
    extra: impl FnOnce(&mut Collector, &mut BTreeSet<&'static str>, &mut Vec<String>) -> Result<(), String>,
) -> Result<Contents, String> {
    let mut c = Collector::new();
    let mut have = BTreeSet::new();
    let mut notes = Vec::new();

    // The host's top-level links (usrmerge: /bin -> usr/bin, ...).
    for top in ["bin", "sbin", "lib", "lib64", "lib32", "libx32"] {
        let host = Path::new("/").join(top);
        if let Ok(target) = std::fs::read_link(&host) {
            c.tree.symlink(top, &target.to_string_lossy());
        }
    }

    match find_program(&["bash"]) {
        Some(p) => {
            c.add(&p)?;
            have.insert("bash");
        }
        None => notes.push("bash: not found on this host".into()),
    }
    let mut missing = Vec::new();
    for name in COREUTILS {
        match find_program(&[name]) {
            Some(p) => c.add(&p)?,
            None => missing.push(*name),
        }
    }
    if missing.iter().any(|m| ["ls", "sort", "head", "seq", "wc", "cat"].contains(m)) {
        notes.push(format!("coreutils: missing {}", missing.join(", ")));
    } else {
        have.insert("coreutils");
        if !missing.is_empty() {
            notes.push(format!("coreutils: left out (not on this host): {}", missing.join(", ")));
        }
    }
    for name in ["grep", "sed"] {
        match find_program(&[name]) {
            Some(p) => {
                c.add(&p)?;
                have.insert(name);
            }
            None => notes.push(format!("{name}: not found on this host")),
        }
    }

    // Python: the interpreter python3 leads to, and its library directory.
    match find_program(&["python3"]).and_then(|p| std::fs::canonicalize(p).ok()) {
        Some(real) => {
            let version = real.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let lib = Path::new("/usr/lib").join(&version);
            if version.starts_with("python3") && lib.join("os.py").is_file() {
                c.add(Path::new("/usr/bin/python3"))?;
                let mut skip: Vec<&str> = PYTHON_SKIP.to_vec();
                let config = std::fs::read_dir(&lib)
                    .map(|d| {
                        d.filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .filter(|n| n.starts_with("config-"))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                skip.extend(config.iter().map(String::as_str));
                c.add_tree(&lib, &skip)?;
                // The image's python3 is a link to the interpreter itself,
                // not to the host's alternatives.
                c.tree.symlink("usr/bin/python3", &version);
                have.insert("python3");
            } else {
                notes.push(format!("python3: {} has no standard library at {}", real.display(), lib.display()));
            }
        }
        None => notes.push("python3: not found on this host".into()),
    }

    // The C compiler.
    match gcc_parts() {
        Ok(parts) => {
            for p in &parts.files {
                c.add(p)?;
            }
            for (link, target) in &parts.links {
                c.tree.symlink(link, target);
            }
            have.insert("gcc");
        }
        Err(why) => notes.push(format!("gcc: {why}")),
    }

    // Mesa's software drivers, GBM and EGL, modetest and kmscube (mesa.rs,
    // ADR 0115).
    crate::mesa::add_to_root(&mut c, &mut have, &mut notes)?;
    // Xwayland and X11 programs with their data (ADR 0118, xwayland.rs).
    crate::xwayland::add_to_root(&mut c, &mut have, &mut notes)?;
    // Mesa with its virgl and Venus drivers, eglinfo, kmscube, vulkaninfo
    // (virgl.rs, ADR 0119).
    crate::virgl::add_to_root(&mut c, &mut have, &mut notes)?;

    extra(&mut c, &mut have, &mut notes)?;
    c.add_needed()?;

    let mut tree = c.tree;
    for (dir, mode) in [
        ("dev", 0o755),
        ("proc", 0o555),
        ("sys", 0o555),
        ("tmp", 0o1777),
        ("root", 0o700),
        ("mnt", 0o755),
        ("run", 0o755),
        ("home", 0o755),
        ("var/tmp", 0o1777),
        ("usr/local/bin", 0o755),
        // Where the kernel keeps the NT registry's hive files (ADR 0066):
        // a disk with a `/poly` directory holds them.
        ("poly/nt/registry", 0o700),
        // The Windows drive C: beside them (ADR 0073): the kernel makes the
        // Windows folders in it and links /nt/c to it.
        ("poly/nt/c", 0o755),
    ] {
        tree.dir(dir, mode);
    }
    tree.file("etc/passwd", 0o644, crate::initramfs::PASSWD.replace("/bin/sh", "/bin/bash").as_bytes());
    tree.file("etc/group", 0o644, crate::initramfs::GROUP.as_bytes());
    tree.file("etc/hostname", 0o644, b"poly\n");
    tree.file("etc/hosts", 0o644, crate::initramfs::HOSTS.as_bytes());
    tree.file("etc/resolv.conf", 0o644, crate::initramfs::RESOLV_CONF.as_bytes());
    tree.file("etc/profile", 0o644, PROFILE.as_bytes());
    tree.file("etc/inputrc", 0o644, INPUTRC.as_bytes());
    tree.file("etc/shells", 0o644, b"/bin/sh\n/bin/bash\n");
    // The font store: fonts.conf, and the bundled fonts once fetched
    // (fonts.rs, ADR 0105).
    match crate::fonts::add_to_root(&mut tree) {
        Ok(true) => {
            have.insert("fonts");
        }
        Ok(false) => notes.push("fonts: not fetched (cargo xtask fonts fetch); /etc/fonts/fonts.conf only".into()),
        Err(e) => notes.push(format!("fonts: {e}")),
    }
    if have.contains("bash") && !tree.entries.contains_key("usr/bin/sh") {
        tree.symlink("usr/bin/sh", "bash");
    }
    // BusyBox's init (pid 1, outside the chroot) powers off on SIGUSR2 and
    // reboots on SIGTERM (ADR 0051).
    tree.file("usr/local/sbin/poweroff", 0o755, POWEROFF.as_bytes());
    tree.file("usr/local/sbin/reboot", 0o755, REBOOT.as_bytes());
    tree.file("root/poly-test.py", 0o644, PYTHON_TEST.as_bytes());
    tree.file("root/hello.c", 0o644, HELLO_C.as_bytes());
    tree.file("root/poly-test-sys.py", 0o644, PYTHON_SYS_TEST.as_bytes());
    tree.file("root/cc/main.c", 0o644, C_MAIN.as_bytes());
    tree.file("root/cc/util.c", 0o644, C_UTIL.as_bytes());
    tree.file("root/cc/util.h", 0o644, C_UTIL_H.as_bytes());
    Ok(Contents { tree, have, notes })
}

/// The compiler's files and the links to make.
struct GccParts {
    files: Vec<PathBuf>,
    links: Vec<(String, String)>,
}

fn gcc_parts() -> Result<GccParts, String> {
    let driver_link = find_program(&["gcc", "cc"]).ok_or("no gcc or cc on this host")?;
    let driver = std::fs::canonicalize(&driver_link).map_err(|e| format!("{}: {e}", driver_link.display()))?;
    let (exec, lib) = gcc_dirs(&driver).ok_or("no cc1 in /usr/libexec/gcc or /usr/lib/gcc")?;
    let mut files = vec![driver.clone()];
    for name in ["as", "ld"] {
        files.push(find_program(&[name]).ok_or_else(|| format!("no {name} (binutils) on this host"))?);
    }
    if let Some(p) = find_program(&["ld.bfd"]) {
        files.push(p);
    }
    for name in ["cc1", "collect2", "lto-wrapper", "liblto_plugin.so"] {
        let p = exec.join(name);
        if p.exists() {
            files.push(p);
        }
    }
    // Start files, libgcc and its linker script, the plugin's other name.
    let entries: Vec<PathBuf> = std::fs::read_dir(&lib)
        .map_err(|e| format!("{}: {e}", lib.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    for p in &entries {
        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let wanted = (n.starts_with("crt") && n.ends_with(".o"))
            || ["libgcc.a", "libgcc_eh.a", "libgcc_s.so", "liblto_plugin.so", "specs"].contains(&n.as_str());
        if wanted {
            files.push(p.clone());
        }
    }
    // The C library's link-time files, next to its linker script libc.so.
    let triple = lib.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let libdir = [format!("/usr/lib/{triple}"), "/usr/lib64".into(), "/usr/lib".into()]
        .into_iter()
        .map(PathBuf::from)
        .find(|d| d.join("libc.so").is_file())
        .ok_or("no libc.so (the C library's development files) on this host")?;
    for name in ["libc.so", "libc_nonshared.a", "crt1.o", "Scrt1.o", "rcrt1.o", "crti.o", "crtn.o", "libm.so"] {
        let p = libdir.join(name);
        if p.exists() {
            files.extend(script_paths(&p).into_iter().filter(|q| q.exists()));
            files.push(p);
        }
    }
    let search = SearchPath::host();
    if let Some((p, _)) = search.find("libgcc_s.so.1", &[]) {
        files.push(p);
    }
    // Headers.
    let mut dirs = vec![lib.join("include"), PathBuf::from("/usr/local/include"), lib.join("include-fixed")];
    dirs.push(Path::new("/usr/include").join(&triple));
    dirs.push(PathBuf::from("/usr/include"));
    dirs.retain(|d| d.is_dir());
    let headers = header_closure(HEADERS, &dirs);
    if !headers.iter().any(|h| h.ends_with("stdio.h")) {
        return Err("no stdio.h in the compiler's include directories".into());
    }
    files.extend(headers);
    let driver_name = driver.file_name().unwrap().to_string_lossy().into_owned();
    let links = ["gcc", "cc"]
        .iter()
        .filter(|&&n| n != driver_name)
        .map(|n| (format!("usr/bin/{n}"), driver_name.clone()))
        .collect();
    Ok(GccParts { files, links })
}
