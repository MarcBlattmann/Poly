//! Load commands: a bounds-checked walk over the raw commands, and typed
//! views of the ones a loader needs.
//!
//! The load commands follow the header directly. Each starts with
//! `cmd` and `cmdsize`; `cmdsize` is a multiple of 8 in a 64-bit file and
//! includes anything that follows the fixed structure (sections, strings).
//! [`LoadCommands`] checks exactly that, and that `ncmds` commands fill
//! `sizeofcmds` bytes exactly. [`Command::parse`] then checks one command's
//! own structure.

use core::fmt;

use crate::bytes::{cstr, u32_le, u64_le};
use crate::consts::*;
use crate::error::{Error, Result};
use crate::header::CpuKind;
use crate::segment::Segment;
use crate::symtab::{DysymtabCommand, SymtabCommand};

/// One load command, as bytes: `cmd`, `cmdsize` and the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadCommand<'a> {
    cmd: u32,
    data: &'a [u8],
}

impl<'a> LoadCommand<'a> {
    /// The command type, `LC_*`.
    pub const fn cmd(&self) -> u32 {
        self.cmd
    }

    /// `cmdsize`: the length of [`data`](Self::data).
    pub const fn cmdsize(&self) -> usize {
        self.data.len()
    }

    /// All bytes of the command, starting with `cmd`.
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Whether dyld must understand the command (`LC_REQ_DYLD`).
    pub const fn is_required_by_dyld(&self) -> bool {
        self.cmd & LC_REQ_DYLD != 0
    }
}

/// Iterator over the raw load commands of a 64-bit file.
///
/// Yields `Err` once and then stops when the commands are inconsistent: a
/// `cmdsize` below 8, not a multiple of 8 or past the end of the area, fewer
/// commands than `ncmds`, or bytes left over after `ncmds` commands.
#[derive(Clone, Debug)]
pub struct LoadCommands<'a> {
    rest: &'a [u8],
    remaining: u32,
    done: bool,
}

impl<'a> LoadCommands<'a> {
    /// Walks `area`, which holds `ncmds` commands.
    pub fn new(area: &'a [u8], ncmds: u32) -> Self {
        LoadCommands { rest: area, remaining: ncmds, done: false }
    }
}

impl<'a> Iterator for LoadCommands<'a> {
    type Item = Result<LoadCommand<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.remaining == 0 {
            self.done = true;
            return if self.rest.is_empty() { None } else { Some(Err(Error::CommandsSizeMismatch)) };
        }
        let item = (|| {
            if self.rest.is_empty() {
                return Err(Error::CommandsSizeMismatch);
            }
            let cmd = u32_le(self.rest, 0).ok_or(Error::BadCommandSize)?;
            let size = u32_le(self.rest, 4).ok_or(Error::BadCommandSize)? as usize;
            if size < 8 || !size.is_multiple_of(8) {
                return Err(Error::BadCommandSize);
            }
            if size > self.rest.len() {
                return Err(Error::CommandOverrun);
            }
            let (data, rest) = self.rest.split_at(size);
            self.rest = rest;
            self.remaining -= 1;
            Ok(LoadCommand { cmd, data })
        })();
        if item.is_err() {
            self.done = true;
        }
        Some(item)
    }
}

/// A version number packed as `xxxx.yy.zz` (major in the high 16 bits).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32);

impl Version {
    /// The major version.
    pub const fn major(&self) -> u32 {
        self.0 >> 16
    }
    /// The minor version.
    pub const fn minor(&self) -> u32 {
        (self.0 >> 8) & 0xff
    }
    /// The patch version.
    pub const fn patch(&self) -> u32 {
        self.0 & 0xff
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major(), self.minor(), self.patch())
    }
}

impl fmt::Debug for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Version({self})")
    }
}

/// `LC_SOURCE_VERSION`: `A.B.C.D.E` packed as `a24.b10.c10.d10.e10`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceVersion(pub u64);

impl SourceVersion {
    /// The five parts `[A, B, C, D, E]`.
    pub const fn parts(&self) -> [u32; 5] {
        [
            (self.0 >> 40) as u32,
            ((self.0 >> 30) & 0x3ff) as u32,
            ((self.0 >> 20) & 0x3ff) as u32,
            ((self.0 >> 10) & 0x3ff) as u32,
            (self.0 & 0x3ff) as u32,
        ]
    }
}

/// How a dynamic library is used by an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DylibKind {
    /// `LC_LOAD_DYLIB`.
    Load,
    /// `LC_LOAD_WEAK_DYLIB`: may be missing.
    Weak,
    /// `LC_REEXPORT_DYLIB`: loaded, and its exports are this image's too.
    Reexport,
    /// `LC_LAZY_LOAD_DYLIB`: loaded at first use.
    Lazy,
    /// `LC_LOAD_UPWARD_DYLIB`: a dependency that depends on this image.
    Upward,
    /// `LC_ID_DYLIB`: this image's own identification, not a dependency.
    Id,
}

/// A `dylib_command`: a library this image loads, or its own identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dylib<'a> {
    /// What the command is.
    pub kind: DylibKind,
    /// The install name or path, e.g. `@rpath/libtiny.dylib`.
    pub name: &'a str,
    /// The build time stamp (0 in files made by current linkers).
    pub timestamp: u32,
    /// The library's current version.
    pub current_version: Version,
    /// The compatibility version the library promises.
    pub compatibility_version: Version,
    /// `DYLIB_USE_*` flags of the alternate `dylib_use_command` encoding
    /// (macOS 15), 0 for the classic encoding.
    pub use_flags: u32,
}

/// What a string-only command is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DylinkerKind {
    /// `LC_LOAD_DYLINKER`: the dynamic linker to run.
    Load,
    /// `LC_ID_DYLINKER`: this image is a dynamic linker.
    Id,
    /// `LC_DYLD_ENVIRONMENT`: a `NAME=value` string for dyld.
    Environment,
}

/// A `dylinker_command`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dylinker<'a> {
    /// What the command is.
    pub kind: DylinkerKind,
    /// The path, or the environment string.
    pub name: &'a str,
}

/// `LC_MAIN`: where `main` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryPoint {
    /// File offset of `main`, from the start of `__TEXT`.
    pub entryoff: u64,
    /// The initial stack size, 0 for the default.
    pub stacksize: u64,
}

/// `LC_BUILD_VERSION`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildVersion<'a> {
    /// `PLATFORM_*`.
    pub platform: u32,
    /// The minimum OS version.
    pub minos: Version,
    /// The SDK version the image was built with.
    pub sdk: Version,
    tools: &'a [u8],
}

/// A tool that built the image (`struct build_tool_version`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildTool {
    /// `TOOL_*`.
    pub tool: u32,
    /// The tool's version.
    pub version: Version,
}

impl BuildVersion<'_> {
    /// The tools listed in the command.
    pub fn tools(&self) -> impl Iterator<Item = BuildTool> + '_ {
        self.tools.chunks_exact(BUILD_TOOL_VERSION_SIZE).map(|c| BuildTool {
            tool: u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
            version: Version(u32::from_le_bytes([c[4], c[5], c[6], c[7]])),
        })
    }
}

/// `LC_VERSION_MIN_*`: the older way to say the minimum OS version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionMin {
    /// The platform the command implies (`PLATFORM_MACOS`, ...).
    pub platform: u32,
    /// The minimum OS version.
    pub version: Version,
    /// The SDK version.
    pub sdk: Version,
}

/// Which `linkedit_data_command` a command is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkeditKind {
    /// `LC_CODE_SIGNATURE`.
    CodeSignature,
    /// `LC_SEGMENT_SPLIT_INFO`.
    SegmentSplitInfo,
    /// `LC_FUNCTION_STARTS`.
    FunctionStarts,
    /// `LC_DATA_IN_CODE`.
    DataInCode,
    /// `LC_DYLIB_CODE_SIGN_DRS`.
    DylibCodeSignDrs,
    /// `LC_LINKER_OPTIMIZATION_HINT`.
    LinkerOptimizationHint,
    /// `LC_DYLD_EXPORTS_TRIE`.
    ExportsTrie,
    /// `LC_DYLD_CHAINED_FIXUPS`.
    ChainedFixups,
    /// `LC_ATOM_INFO`.
    AtomInfo,
    /// `LC_FUNCTION_VARIANTS`.
    FunctionVariants,
    /// `LC_FUNCTION_VARIANT_FIXUPS`.
    FunctionVariantFixups,
    /// `LC_LAZY_LOAD_DYLIB_INFO`.
    LazyLoadDylibInfo,
}

impl LinkeditKind {
    /// The number of kinds.
    pub(crate) const COUNT: usize = 12;

    /// A distinct small number for each kind.
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::CodeSignature => 0,
            Self::SegmentSplitInfo => 1,
            Self::FunctionStarts => 2,
            Self::DataInCode => 3,
            Self::DylibCodeSignDrs => 4,
            Self::LinkerOptimizationHint => 5,
            Self::ExportsTrie => 6,
            Self::ChainedFixups => 7,
            Self::AtomInfo => 8,
            Self::FunctionVariants => 9,
            Self::FunctionVariantFixups => 10,
            Self::LazyLoadDylibInfo => 11,
        }
    }
}

/// A `linkedit_data_command`: a blob in `__LINKEDIT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkeditData {
    /// Which blob it is.
    pub kind: LinkeditKind,
    /// File offset of the blob.
    pub dataoff: u32,
    /// Size of the blob in bytes.
    pub datasize: u32,
}

/// `LC_ENCRYPTION_INFO` or `LC_ENCRYPTION_INFO_64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncryptionInfo {
    /// File offset of the encrypted range.
    pub cryptoff: u32,
    /// Size of the encrypted range.
    pub cryptsize: u32,
    /// Which encryption system; 0 means not encrypted.
    pub cryptid: u32,
}

/// `LC_DYLD_INFO` or `LC_DYLD_INFO_ONLY`: the file ranges of the classic
/// rebase, bind and export streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DyldInfoCommand {
    /// File offset of the rebase opcodes.
    pub rebase_off: u32,
    /// Size of the rebase opcodes.
    pub rebase_size: u32,
    /// File offset of the bind opcodes.
    pub bind_off: u32,
    /// Size of the bind opcodes.
    pub bind_size: u32,
    /// File offset of the weak bind opcodes.
    pub weak_bind_off: u32,
    /// Size of the weak bind opcodes.
    pub weak_bind_size: u32,
    /// File offset of the lazy bind opcodes.
    pub lazy_bind_off: u32,
    /// Size of the lazy bind opcodes.
    pub lazy_bind_size: u32,
    /// File offset of the export trie.
    pub export_off: u32,
    /// Size of the export trie.
    pub export_size: u32,
}

/// One `flavor`, `count` and state of a thread command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadState<'a> {
    /// The state's flavor (`x86_THREAD_STATE64`, `ARM_THREAD_STATE64`, ...).
    pub flavor: u32,
    /// The number of 32-bit words in [`state`](Self::state).
    pub count: u32,
    /// The register state, `count * 4` bytes.
    pub state: &'a [u8],
}

/// `LC_UNIXTHREAD` or `LC_THREAD`: the initial register state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thread<'a> {
    /// Whether the command is `LC_UNIXTHREAD` (the main thread, with a
    /// stack made by the kernel) rather than `LC_THREAD`.
    pub is_unix_thread: bool,
    body: &'a [u8],
}

impl<'a> Thread<'a> {
    fn parse(lc: &LoadCommand<'a>) -> Result<Thread<'a>> {
        let t = Thread { is_unix_thread: lc.cmd() == LC_UNIXTHREAD, body: &lc.data()[8..] };
        for s in t.states() {
            s?;
        }
        Ok(t)
    }

    /// The register states in the command.
    pub fn states(&self) -> ThreadStates<'a> {
        ThreadStates { rest: self.body, done: false }
    }

    /// The register state of the given flavor.
    fn state(&self, flavor: u32) -> Option<ThreadState<'a>> {
        self.states().flatten().find(|s| s.flavor == flavor)
    }

    /// The initial program counter, for the CPU the image is for: `rip` of
    /// `x86_THREAD_STATE64`, `pc` of `ARM_THREAD_STATE64`.
    pub fn entry_pc(&self, cpu: CpuKind) -> Option<u64> {
        match cpu {
            CpuKind::X86_64 => u64_le(self.state(X86_THREAD_STATE64)?.state, 16 * 8),
            CpuKind::Arm64 | CpuKind::Arm64e => u64_le(self.state(ARM_THREAD_STATE64)?.state, 32 * 8),
            CpuKind::Other => None,
        }
    }

    /// The initial stack pointer, 0 meaning "the kernel picks one".
    pub fn stack_pointer(&self, cpu: CpuKind) -> Option<u64> {
        match cpu {
            CpuKind::X86_64 => u64_le(self.state(X86_THREAD_STATE64)?.state, 7 * 8),
            CpuKind::Arm64 | CpuKind::Arm64e => u64_le(self.state(ARM_THREAD_STATE64)?.state, 31 * 8),
            CpuKind::Other => None,
        }
    }
}

/// Iterator over the states of a [`Thread`].
#[derive(Clone, Debug)]
pub struct ThreadStates<'a> {
    rest: &'a [u8],
    done: bool,
}

impl<'a> Iterator for ThreadStates<'a> {
    type Item = Result<ThreadState<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        // A command is padded to a multiple of 8 bytes; zero padding after
        // the last state (fewer bytes than a state header) is not a state.
        if self.done || self.rest.is_empty() || (self.rest.len() < 8 && self.rest.iter().all(|&b| b == 0)) {
            return None;
        }
        let item = (|| {
            let flavor = u32_le(self.rest, 0).ok_or(Error::BadThreadState)?;
            let count = u32_le(self.rest, 4).ok_or(Error::BadThreadState)?;
            let bytes = (count as usize).checked_mul(4).ok_or(Error::BadThreadState)?;
            let end = 8usize.checked_add(bytes).ok_or(Error::BadThreadState)?;
            let state = self.rest.get(8..end).ok_or(Error::BadThreadState)?;
            self.rest = &self.rest[end..];
            Ok(ThreadState { flavor, count, state })
        })();
        if item.is_err() {
            self.done = true;
        }
        Some(item)
    }
}

/// A load command with its structure checked and its fields read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command<'a> {
    /// `LC_SEGMENT_64`.
    Segment(Segment<'a>),
    /// `LC_SYMTAB`.
    Symtab(SymtabCommand),
    /// `LC_DYSYMTAB`.
    Dysymtab(DysymtabCommand),
    /// `LC_LOAD_DYLIB`, `LC_LOAD_WEAK_DYLIB`, `LC_REEXPORT_DYLIB`,
    /// `LC_LAZY_LOAD_DYLIB`, `LC_LOAD_UPWARD_DYLIB` and `LC_ID_DYLIB`.
    Dylib(Dylib<'a>),
    /// `LC_LOAD_DYLINKER`, `LC_ID_DYLINKER` and `LC_DYLD_ENVIRONMENT`.
    Dylinker(Dylinker<'a>),
    /// `LC_MAIN`.
    Main(EntryPoint),
    /// `LC_UNIXTHREAD` and `LC_THREAD`.
    Thread(Thread<'a>),
    /// `LC_UUID`.
    Uuid([u8; 16]),
    /// `LC_BUILD_VERSION`.
    BuildVersion(BuildVersion<'a>),
    /// `LC_VERSION_MIN_MACOSX`, `_IPHONEOS`, `_TVOS`, `_WATCHOS`.
    VersionMin(VersionMin),
    /// `LC_RPATH`: a run path entry.
    Rpath(&'a str),
    /// `LC_SOURCE_VERSION`.
    SourceVersion(SourceVersion),
    /// `LC_TARGET_TRIPLE`.
    TargetTriple(&'a str),
    /// The `linkedit_data_command` family.
    LinkeditData(LinkeditData),
    /// `LC_DYLD_INFO` and `LC_DYLD_INFO_ONLY`.
    DyldInfo(DyldInfoCommand),
    /// `LC_ENCRYPTION_INFO` and `LC_ENCRYPTION_INFO_64`.
    Encryption(EncryptionInfo),
    /// Any other command, unread.
    Other(LoadCommand<'a>),
}

/// The string of an `lc_str` at byte `field` of a command whose fixed
/// structure is `min` bytes long.
fn lc_str<'a>(lc: &LoadCommand<'a>, field: usize, min: usize) -> Result<&'a str> {
    let off = u32_le(lc.data(), field).ok_or(Error::BadCommandString)? as usize;
    if off < min {
        return Err(Error::BadCommandString);
    }
    let bytes = cstr(lc.data(), off, lc.data().len()).ok_or(Error::BadCommandString)?;
    core::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)
}

fn need(lc: &LoadCommand<'_>, size: usize) -> Result<()> {
    if lc.cmdsize() < size { Err(Error::CommandOverrun) } else { Ok(()) }
}

fn word(lc: &LoadCommand<'_>, off: usize) -> Result<u32> {
    u32_le(lc.data(), off).ok_or(Error::CommandOverrun)
}

fn linkedit(lc: &LoadCommand<'_>, kind: LinkeditKind) -> Result<Command<'static>> {
    need(lc, LINKEDIT_DATA_COMMAND_SIZE)?;
    Ok(Command::LinkeditData(LinkeditData { kind, dataoff: word(lc, 8)?, datasize: word(lc, 12)? }))
}

impl<'a> Command<'a> {
    /// Checks the structure of `lc` against `cmdsize` and reads its fields.
    ///
    /// Commands this crate has no view for come back as
    /// [`Command::Other`]. Segments are checked for their own consistency
    /// (`cmdsize` against `nsects`); whether they fit the file is checked
    /// by [`MachO::parse`](crate::MachO::parse).
    pub fn parse(lc: LoadCommand<'a>) -> Result<Command<'a>> {
        Ok(match lc.cmd() {
            LC_SEGMENT_64 => Command::Segment(Segment::parse(&lc)?),
            LC_SYMTAB => {
                need(&lc, SYMTAB_COMMAND_SIZE)?;
                Command::Symtab(SymtabCommand {
                    symoff: word(&lc, 8)?,
                    nsyms: word(&lc, 12)?,
                    stroff: word(&lc, 16)?,
                    strsize: word(&lc, 20)?,
                })
            }
            LC_DYSYMTAB => {
                need(&lc, DYSYMTAB_COMMAND_SIZE)?;
                let mut f = [0u32; 18];
                for (i, v) in f.iter_mut().enumerate() {
                    *v = word(&lc, 8 + i * 4)?;
                }
                Command::Dysymtab(DysymtabCommand::from_words(f))
            }
            LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LAZY_LOAD_DYLIB | LC_LOAD_UPWARD_DYLIB
            | LC_ID_DYLIB => {
                need(&lc, DYLIB_COMMAND_SIZE)?;
                let mut kind = match lc.cmd() {
                    LC_LOAD_DYLIB => DylibKind::Load,
                    LC_LOAD_WEAK_DYLIB => DylibKind::Weak,
                    LC_REEXPORT_DYLIB => DylibKind::Reexport,
                    LC_LAZY_LOAD_DYLIB => DylibKind::Lazy,
                    LC_LOAD_UPWARD_DYLIB => DylibKind::Upward,
                    _ => DylibKind::Id,
                };
                let word12 = word(&lc, 12)?;
                let mut timestamp = word12;
                let mut use_flags = 0;
                let mut min = DYLIB_COMMAND_SIZE;
                if word12 == DYLIB_USE_MARKER {
                    // The alternate encoding: a marker where the time stamp
                    // was, and flags after the versions.
                    need(&lc, 28)?;
                    min = 28;
                    timestamp = 0;
                    use_flags = word(&lc, 24)?;
                    if kind == DylibKind::Load {
                        if use_flags & DYLIB_USE_REEXPORT != 0 {
                            kind = DylibKind::Reexport;
                        } else if use_flags & DYLIB_USE_WEAK_LINK != 0 {
                            kind = DylibKind::Weak;
                        } else if use_flags & DYLIB_USE_UPWARD != 0 {
                            kind = DylibKind::Upward;
                        }
                    }
                }
                Command::Dylib(Dylib {
                    kind,
                    name: lc_str(&lc, 8, min)?,
                    timestamp,
                    current_version: Version(word(&lc, 16)?),
                    compatibility_version: Version(word(&lc, 20)?),
                    use_flags,
                })
            }
            LC_LOAD_DYLINKER | LC_ID_DYLINKER | LC_DYLD_ENVIRONMENT => {
                need(&lc, STR_COMMAND_SIZE)?;
                let kind = match lc.cmd() {
                    LC_LOAD_DYLINKER => DylinkerKind::Load,
                    LC_ID_DYLINKER => DylinkerKind::Id,
                    _ => DylinkerKind::Environment,
                };
                Command::Dylinker(Dylinker { kind, name: lc_str(&lc, 8, STR_COMMAND_SIZE)? })
            }
            LC_MAIN => {
                need(&lc, ENTRY_POINT_COMMAND_SIZE)?;
                Command::Main(EntryPoint {
                    entryoff: u64_le(lc.data(), 8).ok_or(Error::CommandOverrun)?,
                    stacksize: u64_le(lc.data(), 16).ok_or(Error::CommandOverrun)?,
                })
            }
            LC_UNIXTHREAD | LC_THREAD => Command::Thread(Thread::parse(&lc)?),
            LC_UUID => {
                need(&lc, UUID_COMMAND_SIZE)?;
                let mut u = [0u8; 16];
                u.copy_from_slice(&lc.data()[8..24]);
                Command::Uuid(u)
            }
            LC_BUILD_VERSION => {
                need(&lc, BUILD_VERSION_COMMAND_SIZE)?;
                let ntools = word(&lc, 20)? as usize;
                let end = ntools
                    .checked_mul(BUILD_TOOL_VERSION_SIZE)
                    .and_then(|n| n.checked_add(BUILD_VERSION_COMMAND_SIZE))
                    .ok_or(Error::CommandOverrun)?;
                let tools = lc.data().get(BUILD_VERSION_COMMAND_SIZE..end).ok_or(Error::CommandOverrun)?;
                Command::BuildVersion(BuildVersion {
                    platform: word(&lc, 8)?,
                    minos: Version(word(&lc, 12)?),
                    sdk: Version(word(&lc, 16)?),
                    tools,
                })
            }
            LC_VERSION_MIN_MACOSX | LC_VERSION_MIN_IPHONEOS | LC_VERSION_MIN_TVOS | LC_VERSION_MIN_WATCHOS => {
                need(&lc, VERSION_MIN_COMMAND_SIZE)?;
                let platform = match lc.cmd() {
                    LC_VERSION_MIN_MACOSX => PLATFORM_MACOS,
                    LC_VERSION_MIN_IPHONEOS => PLATFORM_IOS,
                    LC_VERSION_MIN_TVOS => PLATFORM_TVOS,
                    _ => PLATFORM_WATCHOS,
                };
                Command::VersionMin(VersionMin {
                    platform,
                    version: Version(word(&lc, 8)?),
                    sdk: Version(word(&lc, 12)?),
                })
            }
            LC_RPATH => {
                need(&lc, STR_COMMAND_SIZE)?;
                Command::Rpath(lc_str(&lc, 8, STR_COMMAND_SIZE)?)
            }
            LC_TARGET_TRIPLE => {
                need(&lc, STR_COMMAND_SIZE)?;
                Command::TargetTriple(lc_str(&lc, 8, STR_COMMAND_SIZE)?)
            }
            LC_SOURCE_VERSION => {
                need(&lc, SOURCE_VERSION_COMMAND_SIZE)?;
                Command::SourceVersion(SourceVersion(u64_le(lc.data(), 8).ok_or(Error::CommandOverrun)?))
            }
            LC_CODE_SIGNATURE => linkedit(&lc, LinkeditKind::CodeSignature)?,
            LC_SEGMENT_SPLIT_INFO => linkedit(&lc, LinkeditKind::SegmentSplitInfo)?,
            LC_FUNCTION_STARTS => linkedit(&lc, LinkeditKind::FunctionStarts)?,
            LC_DATA_IN_CODE => linkedit(&lc, LinkeditKind::DataInCode)?,
            LC_DYLIB_CODE_SIGN_DRS => linkedit(&lc, LinkeditKind::DylibCodeSignDrs)?,
            LC_LINKER_OPTIMIZATION_HINT => linkedit(&lc, LinkeditKind::LinkerOptimizationHint)?,
            LC_DYLD_EXPORTS_TRIE => linkedit(&lc, LinkeditKind::ExportsTrie)?,
            LC_DYLD_CHAINED_FIXUPS => linkedit(&lc, LinkeditKind::ChainedFixups)?,
            LC_ATOM_INFO => linkedit(&lc, LinkeditKind::AtomInfo)?,
            LC_FUNCTION_VARIANTS => linkedit(&lc, LinkeditKind::FunctionVariants)?,
            LC_FUNCTION_VARIANT_FIXUPS => linkedit(&lc, LinkeditKind::FunctionVariantFixups)?,
            LC_LAZY_LOAD_DYLIB_INFO => linkedit(&lc, LinkeditKind::LazyLoadDylibInfo)?,
            LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                need(&lc, DYLD_INFO_COMMAND_SIZE)?;
                Command::DyldInfo(DyldInfoCommand {
                    rebase_off: word(&lc, 8)?,
                    rebase_size: word(&lc, 12)?,
                    bind_off: word(&lc, 16)?,
                    bind_size: word(&lc, 20)?,
                    weak_bind_off: word(&lc, 24)?,
                    weak_bind_size: word(&lc, 28)?,
                    lazy_bind_off: word(&lc, 32)?,
                    lazy_bind_size: word(&lc, 36)?,
                    export_off: word(&lc, 40)?,
                    export_size: word(&lc, 44)?,
                })
            }
            LC_ENCRYPTION_INFO | LC_ENCRYPTION_INFO_64 => {
                need(&lc, ENCRYPTION_INFO_COMMAND_SIZE)?;
                Command::Encryption(EncryptionInfo {
                    cryptoff: word(&lc, 8)?,
                    cryptsize: word(&lc, 12)?,
                    cryptid: word(&lc, 16)?,
                })
            }
            _ => Command::Other(lc),
        })
    }
}
