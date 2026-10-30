//! C++ Windows test programs (ADR 0098): one `.cpp` file each, beside the
//! Rust ones (ADR 0075), for what Rust cannot do: throw and catch C++
//! exceptions through the system's `msvcrt.dll`.
//!
//! ```text
//! <name>.cpp ─ clang++ --target=x86_64-pc-windows-msvc -ffreestanding -nostdlib -fcxx-exceptions ─► <name>.obj
//!               (or -S, the assembly rewritten as the file's `poly-cxx:` line asks, then assembled)
//! tests/windows/cxx/{msvcrt,kernel32}.def ─ llvm-dlltool ─► import libraries
//! <name>.obj + import libraries ─ lld-link /NODEFAULTLIB /ENTRY:mainCRTStartup ─► <name>.exe
//! ```
//!
//! A test includes `tests/windows/cxx/poly_wtest.h` (the observation file,
//! the checks, the entry point) and imports by name from `kernel32.dll` and
//! `msvcrt.dll` only, so the same executable runs on real Windows and on
//! Poly. No Microsoft header, library or SDK is involved: the import
//! libraries come from our `.def` files of exported names.
//!
//! **The `poly-cxx:` line.** clang emits the x64 C++ exception tables of
//! the newest form (`__CxxFrameHandler3`, FuncInfo magic `0x19930522`, EH
//! flags 1 for `/EHs`). Programs built by older compilers name
//! `__CxxFrameHandler` and older magic numbers (7-Zip's `7za.exe` imports
//! `__CxxFrameHandler`). A test asks for those with one line in its
//! source:
//!
//! ```text
//! // poly-cxx: handler=__CxxFrameHandler magic=0x19930520 ehflags=0 cflags=-fasync-exceptions
//! ```
//!
//! `handler=` renames the language-specific handler of every function,
//! `magic=` and `ehflags=` replace the FuncInfo fields clang's assembly
//! marks with the comments `# MagicNumber` and `# EHFlags` (the layout is
//! LLVM's, `llvm/lib/CodeGen/AsmPrinter/WinException.cpp`), and `cflags=`
//! adds compiler options from a short allowed list. A program built that
//! way is what an older compiler's tables look like to the runtime: the
//! fields an older FuncInfo does not have are simply not read.

use crate::util::{self, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The support header and `.def` files, relative to the repository root.
pub const SUPPORT_DIR: &str = "tests/windows/cxx";
/// The DLLs whose import libraries are made from `SUPPORT_DIR/<stem>.def`.
pub const IMPORT_DLLS: &[&str] = &["kernel32", "msvcrt"];
/// The DLLs a C++ test program may import from (by name).
pub const ALLOWED_DLLS: &[&str] = &["ntdll.dll", "kernel32.dll", "msvcrt.dll"];
/// Compiler options a `poly-cxx:` line may add.
pub const ALLOWED_CFLAGS: &[&str] = &["-fasync-exceptions", "-O0", "-O1", "-O2"];
/// What clang emits as the FuncInfo magic number (0x19930522).
const CLANG_MAGIC: u32 = 0x1993_0522;

/// What a test's `poly-cxx:` line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Directives {
    pub handler: Option<String>,
    pub magic: Option<u32>,
    pub ehflags: Option<u32>,
    pub cflags: Vec<String>,
}

impl Directives {
    /// Whether the assembly has to be rewritten.
    pub fn rewrites(&self) -> bool {
        self.handler.is_some() || self.magic.is_some() || self.ehflags.is_some()
    }
}

fn number(v: &str) -> Result<u32> {
    match v.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16),
        None => v.parse(),
    }
    .map_err(|_| format!("poly-cxx: {v:?} is not a number"))
}

/// The `poly-cxx:` line of a source (at most one).
pub fn directives(source: &str) -> Result<Directives> {
    let mut d = Directives::default();
    let mut seen = false;
    for line in source.lines() {
        let Some(at) = line.find("poly-cxx:") else { continue };
        if seen {
            return Err("more than one poly-cxx: line".into());
        }
        seen = true;
        for word in line[at + "poly-cxx:".len()..].split_whitespace() {
            let (key, value) = word.split_once('=').ok_or_else(|| format!("poly-cxx: {word:?} is not key=value"))?;
            match key {
                "handler" => {
                    let ok = ["__CxxFrameHandler", "__CxxFrameHandler2", "__CxxFrameHandler3"];
                    if !ok.contains(&value) {
                        return Err(format!("poly-cxx: handler {value:?} is not one of {}", ok.join(", ")));
                    }
                    d.handler = Some(value.into());
                }
                "magic" => {
                    let m = number(value)?;
                    if !(0x1993_0520..=CLANG_MAGIC).contains(&m) {
                        return Err(format!("poly-cxx: magic {m:#x} is not 0x19930520..0x19930522"));
                    }
                    d.magic = Some(m);
                }
                "ehflags" => d.ehflags = Some(number(value)?),
                "cflags" => {
                    for f in value.split(',') {
                        if !ALLOWED_CFLAGS.contains(&f) {
                            return Err(format!("poly-cxx: cflag {f:?} is not one of {}", ALLOWED_CFLAGS.join(" ")));
                        }
                        d.cflags.push(f.into());
                    }
                }
                _ => return Err(format!("poly-cxx: unknown key {key:?}")),
            }
        }
    }
    Ok(d)
}

/// clang's assembly with the handler, magic number and EH flags replaced.
/// Fails if something asked for is not there to replace.
pub fn rewrite_assembly(asm: &str, d: &Directives) -> Result<String> {
    let mut out = String::with_capacity(asm.len());
    let (mut handlers, mut magics, mut flags) = (0usize, 0usize, 0usize);
    for line in asm.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let comment = body.split_once('#').map(|(_, c)| c.trim());
        let mut new = body.to_string();
        if let Some(h) = &d.handler
            && body.contains("__CxxFrameHandler3")
        {
            // Whole words only: `__CxxFrameHandler3` and nothing longer.
            new = replace_word(&new, "__CxxFrameHandler3", h);
            handlers += 1;
        }
        if comment == Some("MagicNumber")
            && let Some(m) = d.magic
        {
            new = replace_long(&new, m)?;
            magics += 1;
        }
        if comment == Some("EHFlags")
            && let Some(f) = d.ehflags
        {
            new = replace_long(&new, f)?;
            flags += 1;
        }
        out.push_str(&quote_imgrel(&new));
        out.push_str(&line[body.len()..]);
    }
    if d.handler.is_some() && handlers == 0 {
        return Err("poly-cxx: handler= given, but no function uses __CxxFrameHandler3".into());
    }
    if d.magic.is_some() && magics == 0 {
        return Err("poly-cxx: magic= given, but the assembly has no `# MagicNumber` field".into());
    }
    if d.ehflags.is_some() && flags == 0 {
        return Err("poly-cxx: ehflags= given, but the assembly has no `# EHFlags` field".into());
    }
    Ok(out)
}

/// clang prints `sym@IMGREL` without quotes even when the symbol's
/// decorated name contains `@` (`_CTA4PEAUDerived@@@IMGREL`), which its own
/// assembler then reads wrongly; quote such names (`"_CTA4PEAUDerived@@"@IMGREL`).
fn quote_imgrel(line: &str) -> String {
    const SUFFIX: &str = "@IMGREL";
    let symbol_char = |c: char| c.is_ascii_alphanumeric() || "_?$@.".contains(c);
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find(SUFFIX) {
        let head = &rest[..i];
        let start = head.char_indices().rev().take_while(|&(_, c)| symbol_char(c)).last().map_or(i, |(j, _)| j);
        let name = &head[start..];
        let quoted = head[..start].ends_with('"');
        if !quoted && name.contains(['@', '?']) {
            out.push_str(&head[..start]);
            out.push('"');
            out.push_str(name);
            out.push('"');
        } else {
            out.push_str(head);
        }
        out.push_str(SUFFIX);
        rest = &rest[i + SUFFIX.len()..];
    }
    out.push_str(rest);
    out
}

fn replace_word(line: &str, word: &str, with: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find(word) {
        let after = rest[i + word.len()..].chars().next();
        let before = rest[..i].chars().last();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
        out.push_str(&rest[..i]);
        out.push_str(if ident(before) || ident(after) { word } else { with });
        rest = &rest[i + word.len()..];
    }
    out.push_str(rest);
    out
}

/// `\t.long\t<old>   # Comment` with the value replaced.
fn replace_long(line: &str, value: u32) -> Result<String> {
    let (code, comment) = line.split_once('#').unwrap_or((line, ""));
    let t = code.trim_start();
    let Some(old) = t.strip_prefix(".long") else {
        return Err(format!("poly-cxx: expected a .long field: {line:?}"));
    };
    let indent = &code[..code.len() - t.len()];
    let pad = match &old[..old.len() - old.trim_start().len()] {
        "" => " ",
        ws => ws,
    };
    Ok(format!("{indent}.long{pad}{value:<30} #{comment}"))
}

/// The tools: `CLANGXX`, `LLD_LINK`, `LLVM_DLLTOOL`, else these names.
pub struct Tools {
    pub clang: String,
    pub lld_link: String,
    pub dlltool: String,
}

impl Tools {
    pub fn from_env() -> Tools {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
        Tools {
            clang: var("CLANGXX", "clang++"),
            lld_link: var("LLD_LINK", "lld-link"),
            dlltool: var("LLVM_DLLTOOL", "llvm-dlltool"),
        }
    }

    /// `clang++ --version`'s first line.
    pub fn version(&self) -> Result<String> {
        let out = Command::new(&self.clang)
            .arg("--version")
            .output()
            .map_err(|e| format!("{}: {e} (C++ Windows tests need clang++, lld-link and llvm-dlltool)", self.clang))?;
        Ok(String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string())
    }
}

fn run(cmd: &mut Command) -> Result {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// The compiler options of every C++ test: freestanding, no runtime of its
/// own, C++ exceptions and Microsoft extensions (`__try`/`__except`) on,
/// no build-machine paths in the output.
pub fn clang_flags(repo: &Path) -> Vec<String> {
    [
        "--target=x86_64-pc-windows-msvc",
        "-std=c++17",
        "-fms-extensions",
        "-fexceptions",
        "-fcxx-exceptions",
        "-ffreestanding",
        "-nostdlib",
        "-fno-builtin",
        "-O1",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!("-I{}", repo.join(SUPPORT_DIR).display()), format!("-ffile-prefix-map={}=/poly", repo.display())])
    .collect()
}

/// The linker's options: no default libraries, our entry point, a console
/// program, a reproducible image without debug information.
pub const LINK_FLAGS: &[&str] =
    &["/NODEFAULTLIB", "/ENTRY:mainCRTStartup", "/SUBSYSTEM:CONSOLE", "/Brepro", "/DEBUG:NONE", "/MACHINE:X64"];

/// Make the import libraries in `work` from the `.def` files.
pub fn import_libraries(repo: &Path, tools: &Tools, work: &Path) -> Result<Vec<PathBuf>> {
    util::create_dir(work)?;
    let mut libs = Vec::new();
    for dll in IMPORT_DLLS {
        let def = repo.join(SUPPORT_DIR).join(format!("{dll}.def"));
        let lib = work.join(format!("{dll}.lib"));
        run(Command::new(&tools.dlltool).args(["-m", "i386:x86-64", "-d"]).arg(&def).arg("-l").arg(&lib))?;
        libs.push(lib);
    }
    Ok(libs)
}

/// Build `source` into `work/<name>.exe`; returns its path.
pub fn build_one(
    repo: &Path,
    tools: &Tools,
    libs: &[PathBuf],
    name: &str,
    source: &Path,
    work: &Path,
) -> Result<PathBuf> {
    let text = util::read_string(source)?;
    let d = directives(&text).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut flags = clang_flags(repo);
    flags.extend(d.cflags.iter().cloned());
    let obj = work.join(format!("{name}.obj"));
    if d.rewrites() {
        let asm = work.join(format!("{name}.s"));
        run(Command::new(&tools.clang).args(&flags).arg("-S").arg("-o").arg(&asm).arg(source))?;
        let rewritten = rewrite_assembly(&util::read_string(&asm)?, &d).map_err(|e| format!("{name}: {e}"))?;
        let asm2 = work.join(format!("{name}.rewritten.s"));
        util::write(&asm2, rewritten)?;
        run(Command::new(&tools.clang).args(["--target=x86_64-pc-windows-msvc", "-c", "-o"]).arg(&obj).arg(&asm2))?;
    } else {
        run(Command::new(&tools.clang).args(&flags).arg("-c").arg("-o").arg(&obj).arg(source))?;
    }
    let exe = work.join(format!("{name}.exe"));
    run(Command::new(&tools.lld_link).args(LINK_FLAGS).arg(format!("/OUT:{}", exe.display())).arg(&obj).args(libs))?;
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directive_lines() {
        assert_eq!(directives("int x;\n").unwrap(), Directives::default());
        let d = directives(
            "// poly-cxx: handler=__CxxFrameHandler magic=0x19930520 ehflags=0 cflags=-fasync-exceptions,-O0\n",
        )
        .unwrap();
        assert_eq!(d.handler.as_deref(), Some("__CxxFrameHandler"));
        assert_eq!((d.magic, d.ehflags), (Some(0x1993_0520), Some(0)));
        assert_eq!(d.cflags, ["-fasync-exceptions", "-O0"]);
        assert!(d.rewrites());
        assert!(!directives("// poly-cxx: cflags=-O2\n").unwrap().rewrites());
        assert!(directives("// poly-cxx: handler=foo\n").is_err());
        assert!(directives("// poly-cxx: magic=0x12345678\n").is_err());
        assert!(directives("// poly-cxx: cflags=-fplugin=x\n").is_err());
        assert!(directives("// poly-cxx: what=1\n").is_err());
        assert!(directives("// poly-cxx: magic\n").is_err());
        assert!(directives("// poly-cxx: magic=0x19930521\n// poly-cxx: ehflags=0\n").is_err());
    }

    const ASM: &str = "\t.seh_handler __CxxFrameHandler3, @unwind, @except\n\
        \tcallq\t__CxxFrameHandler3x\n\
        \t.long\t429065506                       # MagicNumber\n\
        \t.long\t2                               # MaxState\n\
        \t.long\t1                               # EHFlags\n";

    #[test]
    fn assembly_rewrites() {
        let d = Directives {
            handler: Some("__CxxFrameHandler".into()),
            magic: Some(0x1993_0520),
            ehflags: Some(0),
            cflags: vec![],
        };
        let out = rewrite_assembly(ASM, &d).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "\t.seh_handler __CxxFrameHandler, @unwind, @except");
        assert_eq!(lines[1], "\tcallq\t__CxxFrameHandler3x", "only whole words");
        assert!(lines[2].starts_with("\t.long\t429065504 ") && lines[2].ends_with("# MagicNumber"), "{}", lines[2]);
        assert_eq!(lines[3], "\t.long\t2                               # MaxState");
        assert!(lines[4].starts_with("\t.long\t0 ") && lines[4].ends_with("# EHFlags"), "{}", lines[4]);
        assert_eq!(rewrite_assembly(ASM, &Directives::default()).unwrap(), ASM);
        assert_eq!(quote_imgrel("\t.long\t_CTA4PEAUDerived@@@IMGREL\n"), "\t.long\t\"_CTA4PEAUDerived@@\"@IMGREL\n");
        assert_eq!(quote_imgrel("\t.long\t(\"$tryMap$f\")@IMGREL"), "\t.long\t(\"$tryMap$f\")@IMGREL");
        assert_eq!(quote_imgrel("\t.long\t.Ltmp0@IMGREL+1"), "\t.long\t.Ltmp0@IMGREL+1");
        assert_eq!(quote_imgrel("\t.long\t\"??_R0H@8\"@IMGREL"), "\t.long\t\"??_R0H@8\"@IMGREL");

        let none = "\tretq\n";
        assert!(rewrite_assembly(none, &d).unwrap_err().contains("no function uses"));
        let only_magic = Directives { magic: Some(0x1993_0521), ..Directives::default() };
        assert!(rewrite_assembly(none, &only_magic).unwrap_err().contains("MagicNumber"));
        let only_flags = Directives { ehflags: Some(0), ..Directives::default() };
        assert!(rewrite_assembly(none, &only_flags).unwrap_err().contains("EHFlags"));
    }

    #[test]
    fn flags_are_freestanding_and_path_free() {
        let f = clang_flags(Path::new("/r"));
        for want in ["--target=x86_64-pc-windows-msvc", "-ffreestanding", "-nostdlib", "-fcxx-exceptions"] {
            assert!(f.iter().any(|x| x == want), "{want}");
        }
        assert!(f.contains(&"-I/r/tests/windows/cxx".to_string()));
        assert!(f.contains(&"-ffile-prefix-map=/r=/poly".to_string()));
        assert!(LINK_FLAGS.contains(&"/Brepro") && LINK_FLAGS.contains(&"/NODEFAULTLIB"));
    }
}
