// --- Primitive Type Aliases ---
// Based on lfs.h and lfs.rs
pub type LfsSize = u32;
pub type LfsOff = u32;
pub type LfsSSize = i32;
pub type LfsSOff = i32;
pub type LfsBlock = u32;

// --- Constants ---
// From LFS_NAME_MAX in lfs.h
pub const LFS_NAME_MAX: usize = 255;

// --- Enums ---

/// Possible error codes, these are negative to allow valid positive return values.
/// Based on enum lfs_error in lfs.h and lfs.rs.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LfsError {
    Ok = 0,         // No error
    Io = -5,        // Error during device operation
    Corrupt = -52,  // Corrupted
    NoEnt = -2,     // No directory entry
    Exists = -17,   // Entry already exists
    NotDir = -20,   // Entry is not a dir
    IsDir = -21,    // Entry is a dir
    Inval = -22,    // Invalid parameter
    NoSpc = -28,    // No space left on device
    NoMem = -12,    // No more memory available
}

/// File types.
/// Based on enum lfs_type in lfs.h and lfs.rs.
#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LfsType {
    Reg = 0x11,
    Dir = 0x22,
    Superblock = 0xe2, // Value from lfs.c, lfs.rs has 0xE2 = 226
}

/// File open flags.
/// Based on enum lfs_open_flags in lfs.h and lfs.rs.
/// These are typically used as bitflags.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LfsOpenFlags;

impl LfsOpenFlags {
    // open flags
    pub const RDONLY: u32 = 1;   // Open a file as read only
    pub const WRONLY: u32 = 2;   // Open a file as write only
    pub const RDWR: u32 = 3;     // Open a file as read and write
    pub const CREAT: u32 = 0x0100; // Create a file if it does not exist
    pub const EXCL: u32 = 0x0200;  // Fail if a file already exists
    pub const TRUNC: u32 = 0x0400; // Truncate the existing file to zero size
    pub const APPEND: u32 = 0x0800;// Move to end of file on every write

    // internally used flags
    pub const F_DIRTY: u32 = 0x10000;  // File does not match storage
    pub const F_WRITING: u32 = 0x20000;// File has been written since last flush
    pub const F_READING: u32 = 0x40000;// File has been read since last flush
}

/// File seek flags.
/// Based on enum lfs_whence_flags in lfs.h and lfs.rs.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LfsWhenceFlags {
    Set = 0, // Seek relative to an absolute position
    Cur = 1, // Seek relative to the current file position
    End = 2, // Seek relative to the end of the file
}

// --- Structs ---

/// Configuration provided during initialization of the littlefs.
/// Based on struct lfs_config in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsConfig {

    /// Read a region in a block.
    pub read: Option<
        fn(
            config: *const LfsConfig,
            block: LfsBlock,
            off: LfsOff,
            buffer: *mut u8,
            size: LfsSize,
        ) -> i32,
    >,

    /// Program a region in a block.
    pub prog: Option<
        fn(
            config: *const LfsConfig,
            block: LfsBlock,
            off: LfsOff,
            buffer: *const u8,
            size: LfsSize,
        ) -> i32,
    >,

    /// Erase a block.
    pub erase: Option<fn(config: *const LfsConfig, block: LfsBlock) -> i32>,

    /// Sync the state of the underlying block device.
    pub sync: Option<fn(config: *const LfsConfig) -> i32>,

    pub read_size: LfsSize,
    pub prog_size: LfsSize,
    pub block_size: LfsSize,
    pub block_count: LfsSize,
    pub lookahead: LfsSize,

    /// Optional, statically allocated read buffer.
    pub read_buffer: *mut u8, // void*
    /// Optional, statically allocated program buffer.
    pub prog_buffer: *mut u8, // void*
    /// Optional, statically allocated lookahead buffer. (uint32_t* in C)
    pub lookahead_buffer: *mut u32, // void*
    /// Optional, statically allocated buffer for files.
    pub file_buffer: *mut u8, // void*
}

/// File info structure.
/// Based on struct lfs_info in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct LfsInfo {
    /// Type of the file, either LFS_TYPE_REG or LFS_TYPE_DIR.
    pub type_0: u8, // Actually LfsType
    /// Size of the file, only valid for REG files.
    pub size: LfsSize,
    /// Name of the file stored as a C-style null-terminated string.
    pub name: [u8; LFS_NAME_MAX + 1],
}

/// Union part of LfsDiskEntry.
/// Based on union u in struct lfs_disk_entry from lfs.h and C2RustUnnamed in lfs.rs.
#[repr(C)]
#[derive(Copy, Clone)]
pub union LfsDiskEntryUnion {
    pub file: LfsDiskEntryFile,
    pub dir: [LfsBlock; 2],
}


impl std::fmt::Debug for LfsDiskEntryUnion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // For safety, just print a placeholder for the union
        write!(f, "LfsDiskEntryUnion {{ ... }}")
    }
}

/// File-specific part of LfsDiskEntryUnion.
/// Based on struct file in union u from lfs.h.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsDiskEntryFile {
    pub head: LfsBlock,
    pub size: LfsSize,
}

/// On-disk directory entry structure.
/// Based on struct lfs_disk_entry in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsDiskEntry {
    pub type_0: u8, // Actually LfsType
    pub elen: u8,   // length of attributes
    pub alen: u8,   // length of embedded attributes
    pub nlen: u8,   // length of name
    pub u: LfsDiskEntryUnion,
}

/// In-memory representation of a directory entry.
/// Based on typedef struct lfs_entry lfs_entry_t in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsEntry {
    pub off: LfsOff,
    pub d: LfsDiskEntry,
}
pub type LfsEntryTypeAlias = LfsEntry; // Original C code had lfs_entry_t

/// In-memory cache structure.
/// Based on typedef struct lfs_cache lfs_cache_t in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsCache {
    pub block: LfsBlock,
    pub off: LfsOff,
    pub buffer: *mut u8,
}
pub type LfsCacheTypeAlias = LfsCache;

/// In-memory file structure.
/// Based on typedef struct lfs_file lfs_file_t in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsFile {
    pub next: *mut LfsFile, // Represents a linked list pointer
    pub pair: [LfsBlock; 2],
    pub poff: LfsOff, // parent offset

    pub head: LfsBlock,
    pub size: LfsSize,

    pub flags: u32, // LfsOpenFlags
    pub pos: LfsOff,
    pub block: LfsBlock, // current block
    pub off: LfsOff,   // current offset in block
    pub cache: LfsCache,
}
pub type LfsFileTypeAlias = LfsFile;

/// On-disk directory structure header.
/// Based on struct lfs_disk_dir in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsDiskDir {
    pub rev: u32,
    pub size: LfsSize,
    pub tail: [LfsBlock; 2],
}

/// In-memory directory structure.
/// Based on typedef struct lfs_dir lfs_dir_t in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsDir {
    pub pair: [LfsBlock; 2],
    pub off: LfsOff, // Offset of currently selected entry in the directory block

    pub head: [LfsBlock; 2], // pair of the head directory block
    pub pos: LfsOff,         // Current position for lfs_dir_read / lfs_dir_seek

    pub d: LfsDiskDir,
}
pub type LfsDirTypeAlias = LfsDir;

/// On-disk superblock structure.
/// Based on struct lfs_disk_superblock in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsDiskSuperblock {
    pub type_0: u8, // Actually LfsType
    pub elen: u8,
    pub alen: u8,
    pub nlen: u8,
    pub root: [LfsBlock; 2],
    pub block_size: u32,
    pub block_count: u32,
    pub version: u32,
    pub magic: [u8; 8],
}

/// In-memory superblock structure.
/// Based on typedef struct lfs_superblock lfs_superblock_t in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsSuperblock {
    pub off: LfsOff,
    pub d: LfsDiskSuperblock,
}
pub type LfsSuperblockTypeAlias = LfsSuperblock;

/// Free list structure.
/// Based on typedef struct lfs_free lfs_free_t in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsFree {
    pub end: LfsBlock,
    pub start: LfsBlock,
    pub off: LfsBlock,
    pub lookahead: *mut u32,
}
pub type LfsFreeTypeAlias = LfsFree;

/// Top-level littlefs state structure.
/// Based on typedef struct lfs lfs_t in lfs.h and lfs.rs.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Lfs {
    pub cfg: *const LfsConfig,

    pub root: [LfsBlock; 2],
    pub files: *mut LfsFile, // Head of linked list of open files
    pub deorphaned: bool,

    pub rcache: LfsCache,
    pub pcache: LfsCache,

    pub free: LfsFree,
}
pub type LfsTypeAlias = Lfs;

/// Helper struct for directory commits (from lfs.c).
/// Based on struct lfs_region in lfs.c.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct LfsRegion {
    pub oldoff: LfsOff,
    pub oldlen: LfsSize,
    pub newdata: *const u8, // const void*
    pub newlen: LfsSize,
}