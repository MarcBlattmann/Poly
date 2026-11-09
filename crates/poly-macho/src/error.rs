//! The error type of the whole crate.

use core::fmt;

/// Why a file, or a part of it, was rejected.
///
/// Every variant is a fact about the input, never about the reader: the
/// crate has no internal limits that make valid files fail, except the ones
/// named in the variants (`TooMany…`, `…TooLong`, `…TooDeep`), whose bounds
/// are public constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    // ---- file type -------------------------------------------------------
    /// The file is too short for what it claims to hold, or an offset and
    /// size point outside it.
    Truncated,
    /// The first word is not a Mach-O or universal magic.
    NotMachO,
    /// A 32-bit Mach-O file (`MH_MAGIC`). Only 64-bit files are read.
    Unsupported32Bit,
    /// A big-endian Mach-O file (`MH_CIGAM`, `MH_CIGAM_64`). Only
    /// little-endian files are read.
    UnsupportedBigEndian,
    /// The file is universal; pick a slice with [`Fat`](crate::Fat) first.
    FatFile,
    /// A fat file has no slice for the requested CPU.
    NoMatchingArch,

    // ---- universal files -------------------------------------------------
    /// More than [`MAX_FAT_ARCHES`](crate::MAX_FAT_ARCHES) slices (this also
    /// rejects Java class files, which share the magic).
    TooManyArches,
    /// A slice has size 0.
    EmptySlice,
    /// A slice overlaps the fat header, the slice table or another slice.
    SlicesOverlap,
    /// A slice's alignment exponent is 64 or more.
    BadSliceAlign,

    // ---- header and load commands ---------------------------------------
    /// `sizeofcmds` and `ncmds` do not describe the same bytes.
    CommandsSizeMismatch,
    /// A load command has `cmdsize` below 8 or not a multiple of 8.
    BadCommandSize,
    /// A load command (or something it contains) runs past its `cmdsize`,
    /// or its `cmdsize` is smaller than the command's structure.
    CommandOverrun,
    /// A string offset of a load command is outside the command, or the
    /// string has no terminating NUL inside it.
    BadCommandString,
    /// A path in a load command is not UTF-8.
    InvalidUtf8,
    /// More than [`MAX_SEGMENTS`](crate::MAX_SEGMENTS) `LC_SEGMENT_64` commands.
    TooManySegments,
    /// A segment's addresses or file range wrap around 2^64.
    SegmentOverflow,
    /// A segment's `filesize` exceeds its `vmsize`.
    SegmentFileSizeExceedsVmSize,
    /// A segment's file range lies (partly) outside the file.
    SegmentOutsideFile,
    /// Two segments overlap in memory.
    SegmentsOverlapInMemory,
    /// Two segments overlap in the file.
    SegmentsOverlapInFile,
    /// `nsects` and `cmdsize` of a segment disagree.
    BadSectionCount,
    /// A section lies outside its segment, in memory or in the file.
    SectionOutsideSegment,
    /// A thread command's flavor and count do not fit its size.
    BadThreadState,
    /// The entry point (`LC_MAIN` or the program counter of
    /// `LC_UNIXTHREAD`) is not inside a segment of the image.
    BadEntry,

    // ---- tables in __LINKEDIT -------------------------------------------
    /// A table (symbols, strings, indirect table, linkedit data, dyld info
    /// stream) lies outside the file.
    TableOutsideFile,
    /// A string index is outside the string table.
    StringOutOfRange,
    /// A string has no terminating NUL before the end of its table, or is
    /// longer than [`MAX_NAME_LEN`](crate::MAX_NAME_LEN).
    BadString,
    /// A symbol range of `LC_DYSYMTAB` exceeds the symbol table.
    BadSymbolRange,
    /// A symbol table entry index is out of range.
    SymbolOutOfRange,

    // ---- LEB128 and opcode streams --------------------------------------
    /// A LEB128 number is longer than 10 bytes, runs off the end of its
    /// stream, or does not fit 64 bits.
    BadLeb128,
    /// An opcode stream ends in the middle of an opcode.
    UnexpectedEndOfStream,
    /// A rebase or bind opcode that cannot be applied: no segment set, a
    /// reserved value, or a threaded-bind opcode (not supported).
    BadOpcode(u8),
    /// A rebase or bind names a segment that does not exist.
    BadSegmentIndex,
    /// A rebase or bind lies outside its segment.
    FixupOutsideSegment,
    /// A rebase or bind stream emits more records than its segments could
    /// hold pointers.
    TooManyFixups,

    // ---- export trie -----------------------------------------------------
    /// A trie node offset is outside the trie.
    TrieOutOfRange,
    /// The trie is deeper than [`MAX_TRIE_DEPTH`](crate::MAX_TRIE_DEPTH), or
    /// has more nodes visited than bytes (a cycle or a shared subtree).
    TrieTooComplex,
    /// A symbol name is longer than [`MAX_EXPORT_NAME_LEN`](crate::MAX_EXPORT_NAME_LEN).
    NameTooLong,

    // ---- chained fixups --------------------------------------------------
    /// `fixups_version` is not 0.
    UnsupportedFixupsVersion,
    /// The symbol strings are compressed (`symbols_format` 1).
    CompressedSymbols,
    /// `imports_format` is not one of the three known formats.
    UnsupportedImportsFormat,
    /// A pointer format this crate cannot walk.
    UnsupportedPointerFormat(u16),
    /// `page_size` is not 0x1000 or 0x4000.
    BadPageSize,
    /// A `starts_in_segment` record is smaller than its page count needs or
    /// lies outside the payload.
    BadStartsInSegment,
    /// A `page_start` value is out of range for its page or format.
    BadPageStart,
    /// A fixup chain leaves its page or its segment's file bytes.
    ChainEscapesPage,
    /// An import's library ordinal or name offset is out of range.
    BadImport,
    /// A bind names an import beyond the import table.
    ImportOutOfRange,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Truncated => "file or table truncated",
            Self::NotMachO => "not a Mach-O file",
            Self::Unsupported32Bit => "32-bit Mach-O files are not supported",
            Self::UnsupportedBigEndian => "big-endian Mach-O files are not supported",
            Self::FatFile => "universal file: pick a slice first",
            Self::NoMatchingArch => "no slice for that CPU",
            Self::TooManyArches => "too many slices in universal file",
            Self::EmptySlice => "empty slice in universal file",
            Self::SlicesOverlap => "slices of a universal file overlap",
            Self::BadSliceAlign => "bad slice alignment",
            Self::CommandsSizeMismatch => "sizeofcmds and ncmds disagree",
            Self::BadCommandSize => "bad load command size",
            Self::CommandOverrun => "load command overruns its size",
            Self::BadCommandString => "bad string in load command",
            Self::InvalidUtf8 => "path in load command is not UTF-8",
            Self::TooManySegments => "too many segments",
            Self::SegmentOverflow => "segment range wraps around",
            Self::SegmentFileSizeExceedsVmSize => "segment filesize exceeds vmsize",
            Self::SegmentOutsideFile => "segment lies outside the file",
            Self::SegmentsOverlapInMemory => "segments overlap in memory",
            Self::SegmentsOverlapInFile => "segments overlap in the file",
            Self::BadSectionCount => "segment section count and size disagree",
            Self::SectionOutsideSegment => "section lies outside its segment",
            Self::BadThreadState => "bad thread state",
            Self::BadEntry => "entry point outside the image",
            Self::TableOutsideFile => "table lies outside the file",
            Self::StringOutOfRange => "string index out of range",
            Self::BadString => "unterminated or overlong string",
            Self::BadSymbolRange => "symbol range exceeds the symbol table",
            Self::SymbolOutOfRange => "symbol index out of range",
            Self::BadLeb128 => "bad LEB128 number",
            Self::UnexpectedEndOfStream => "opcode stream ends inside an opcode",
            Self::BadOpcode(_) => "opcode that cannot be applied",
            Self::BadSegmentIndex => "segment index out of range",
            Self::FixupOutsideSegment => "fixup lies outside its segment",
            Self::TooManyFixups => "stream emits too many fixups",
            Self::TrieOutOfRange => "export trie offset out of range",
            Self::TrieTooComplex => "export trie too deep or cyclic",
            Self::NameTooLong => "export name too long",
            Self::UnsupportedFixupsVersion => "unsupported chained fixups version",
            Self::CompressedSymbols => "compressed chained fixups symbols",
            Self::UnsupportedImportsFormat => "unsupported chained imports format",
            Self::UnsupportedPointerFormat(_) => "unsupported chained pointer format",
            Self::BadPageSize => "bad chained fixups page size",
            Self::BadStartsInSegment => "bad starts_in_segment record",
            Self::BadPageStart => "bad chained fixups page start",
            Self::ChainEscapesPage => "fixup chain leaves its page",
            Self::BadImport => "bad chained import",
            Self::ImportOutOfRange => "import ordinal out of range",
        };
        f.write_str(s)
    }
}

/// The result type of this crate.
pub type Result<T> = core::result::Result<T, Error>;
