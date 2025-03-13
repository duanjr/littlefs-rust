/*
 * The little filesystem
 *
 * Copyright (c) 2022, The littlefs authors.
 * Copyright (c) 2017, Arm Limited. All rights reserved.
 * SPDX-License-Identifier: BSD-3-Clause
 */
 use core::ffi::c_void;
 
 /// 版本信息
 
 // 软件库版本
 // 高16位为主版本号，在向后不兼容变更时递增
 // 低16位为次版本号，在添加功能时递增
 pub const LFS_VERSION: u32 = 0x0002000a;
 pub const LFS_VERSION_MAJOR: u32 = 0xffff & (LFS_VERSION >> 16);
 pub const LFS_VERSION_MINOR: u32 = 0xffff & (LFS_VERSION >> 0);
 
 // 磁盘数据结构版本
 // 高16位为主版本号，在向后不兼容变更时递增
 // 低16位为次版本号，在添加功能时递增
 pub const LFS_DISK_VERSION: u32 = 0x00020001;
 pub const LFS_DISK_VERSION_MAJOR: u32 = 0xffff & (LFS_DISK_VERSION >> 16);
 pub const LFS_DISK_VERSION_MINOR: u32 = 0xffff & (LFS_DISK_VERSION >> 0);
 
 /// 基本类型定义
 pub type LfsSize = u32;
 pub type LfsOff = u32;
 pub type LfsSsize = i32;
 pub type LfsSoff = i32;
 pub type LfsBlock = u32;
 
 /// 最大名称大小（字节），可重新定义以减小info结构体的大小
 /// 限制为 <= 1022。存储在超级块中，其他littlefs驱动程序必须遵守
 pub const LFS_NAME_MAX: usize = 255;
 
 /// 文件最大大小（字节），可重新定义以支持其他驱动程序
 /// 在磁盘上限制为 <= 2147483647。存储在超级块中，其他littlefs驱动程序必须遵守
 pub const LFS_FILE_MAX: u32 = 2147483647;
 
 /// 自定义属性的最大大小（字节），可重新定义，但使用较小的LFS_ATTR_MAX没有实际好处
 /// 限制为 <= 1022。存储在超级块中，其他littlefs驱动程序必须遵守
 pub const LFS_ATTR_MAX: usize = 1022;
 
 /// 可能的错误代码，这些是负数以允许有效的正返回值
 #[repr(i32)]
 #[derive(Debug, PartialEq, Eq, Clone, Copy)]
 pub enum LfsError {
     Ok = 0,         // 没有错误
     Io = -5,        // 设备操作期间出错
     Corrupt = -84,  // 已损坏
     NoEnt = -2,     // 没有目录条目
     Exist = -17,    // 条目已存在
     NotDir = -20,   // 条目不是目录
     IsDir = -21,    // 条目是目录
     NotEmpty = -39, // 目录不为空
     BadF = -9,      // 错误的文件编号
     FBig = -27,     // 文件太大
     Inval = -22,    // 无效参数
     NoSpc = -28,    // 设备上没有剩余空间
     NoMem = -12,    // 没有更多可用内存
     NoAttr = -61,   // 没有可用的数据/属性
     NameTooLong = -36, // 文件名太长
 }
 
 /// 文件类型
 #[repr(u16)]
 #[derive(Debug, PartialEq, Eq, Clone, Copy)]
 pub enum LfsType {
     // 文件类型
     Reg = 0x001,
     Dir = 0x002,
 
     // 内部使用的类型
     Splice = 0x400,
     Name = 0x000,
     Struct = 0x200,
     UserAttr = 0x300,
     From = 0x100,
     Tail = 0x600,
     Globals = 0x700,
     Crc = 0x500,
 
     // 内部使用的类型特化
     Create = 0x401,
     Delete = 0x4ff,
     SuperBlock = 0x0ff,
     DirStruct = 0x200,
     CtzStruct = 0x202,
     InlineStruct = 0x201,
     SoftTail = 0x600,
     HardTail = 0x601,
     MoveState = 0x7ff,
     CCrc = 0x500,
     FCrc = 0x5ff,
 
     // 内部芯片源
     FromNoop = 0x000,
     FromMove = 0x101,
     FromUserAttrs = 0x102,
 }
 
 /// 文件打开标志
 #[repr(u32)]
 #[derive(Debug, PartialEq, Eq, Clone, Copy)]
 pub enum LfsOpenFlags {
     // 打开标志
     RdOnly = 1,        // 以只读方式打开文件
     WrOnly = 2,        // 以只写方式打开文件
     RdWr = 3,          // 以读写方式打开文件
     Creat = 0x0100,    // 如果文件不存在则创建
     Excl = 0x0200,     // 如果文件已存在则失败
     Trunc = 0x0400,    // 将现有文件截断为零大小
     Append = 0x0800,   // 每次写入时移动到文件末尾
 
     // 内部使用的标志
     Dirty = 0x010000,   // 文件与存储不匹配
     Writing = 0x020000, // 自上次刷新以来文件已被写入
     Reading = 0x040000, // 自上次刷新以来文件已被读取
     Erred = 0x080000,   // 写入期间发生错误
     Inline = 0x100000,  // 当前内联在目录条目中
 }
 
 /// 文件查找标志
 #[repr(i32)]
 #[derive(Debug, PartialEq, Eq, Clone, Copy)]
 pub enum LfsWhenceFlags {
     Set = 0,   // 相对于绝对位置查找
     Cur = 1,   // 相对于当前文件位置查找
     End = 2,   // 相对于文件末尾查找
 }
 
 /// 文件系统配置
 #[derive(Debug)]
 pub struct LfsConfig {
     // 用户提供的上下文，可用于向块设备操作传递信息
     pub context: *mut c_void,
 
     // 读取块中的区域。负错误代码传递给用户
     pub read: Option<unsafe extern "C" fn(c: *const LfsConfig, block: LfsBlock, 
         off: LfsOff, buffer: *mut c_void, size: LfsSize) -> i32>,
 
     // 写入块中的区域。该块必须已先被擦除。负错误代码传递给用户
     // 如果块应被视为坏块，可能返回LFS_ERR_CORRUPT
     pub prog: Option<unsafe extern "C" fn(c: *const LfsConfig, block: LfsBlock, 
         off: LfsOff, buffer: *const c_void, size: LfsSize) -> i32>,
 
     // 擦除块。必须先擦除块，然后才能写入。已擦除块的状态未定义
     // 负错误代码传递给用户。如果块应被视为坏块，可能返回LFS_ERR_CORRUPT
     pub erase: Option<unsafe extern "C" fn(c: *const LfsConfig, block: LfsBlock) -> i32>,
 
     // 同步底层块设备的状态。负错误代码传递给用户
     pub sync: Option<unsafe extern "C" fn(c: *const LfsConfig) -> i32>,
 
     // 锁定底层块设备。负错误代码传递给用户（仅在启用线程安全时）
     pub lock: Option<unsafe extern "C" fn(c: *const LfsConfig) -> i32>,
 
     // 解锁底层块设备。负错误代码传递给用户（仅在启用线程安全时）
     pub unlock: Option<unsafe extern "C" fn(c: *const LfsConfig) -> i32>,
 
     // 块读取的最小大小（字节）。所有读取操作都是此值的倍数
     pub read_size: LfsSize,
 
     // 块写入的最小大小（字节）。所有写入操作都是此值的倍数
     pub prog_size: LfsSize,
 
     // 可擦除块的大小（字节）。这不影响内存消耗，可能大于物理擦除大小
     // 但是，非内联文件至少占用一个块。必须是读取和写入大小的倍数
     pub block_size: LfsSize,
 
     // 设备上可擦除块的数量。为零时，默认使用存储在磁盘上的块数量
     pub block_count: LfsSize,
 
     // littlefs在驱逐元数据日志并将元数据移动到另一个块之前的擦除周期数
     // 建议值在100-1000范围内，较大的值性能更好，但磨损分布一致性较差
     // 设置为-1以禁用块级磨损均衡
     pub block_cycles: i32,
 
     // 块缓存大小（字节）。每个缓存在RAM中缓冲块的一部分
     // littlefs需要一个读缓存、一个写缓存和每个文件一个额外的缓存
     // 更大的缓存可通过存储更多数据并减少磁盘访问次数来提高性能
     // 必须是读取和写入大小的倍数，以及块大小的因子
     pub cache_size: LfsSize,
 
     // 前瞻缓冲区大小（字节）。更大的前瞻缓冲区可在分配过程中找到更多块
     // 前瞻缓冲区存储为紧凑位图，因此每字节RAM可跟踪8个块
     pub lookahead_size: LfsSize,
 
     // lfs_fs_gc期间元数据压缩的阈值（字节）。超过此阈值的元数据对将在lfs_fs_gc期间被压缩
     // 为零时默认为~88%块大小，但未来可能会更改
     // 设置为-1以在lfs_fs_gc期间禁用元数据压缩
     pub compact_thresh: LfsSize,
 
     // 可选的静态分配读缓冲区。必须为cache_size大小
     // 默认使用lfs_malloc分配此缓冲区
     pub read_buffer: *mut c_void,
 
     // 可选的静态分配写缓冲区。必须为cache_size大小
     // 默认使用lfs_malloc分配此缓冲区
     pub prog_buffer: *mut c_void,
 
     // 可选的静态分配前瞻缓冲区。必须为lookahead_size大小
     // 默认使用lfs_malloc分配此缓冲区
     pub lookahead_buffer: *mut c_void,
 
     // 文件名长度的可选上限（字节）。较大名称的唯一缺点是info结构体的大小
     // 由LFS_NAME_MAX定义控制。为零时默认为LFS_NAME_MAX或存储在磁盘上的name_max
     pub name_max: LfsSize,
 
     // 文件大小的可选上限（字节）。较大文件没有缺点，但必须 <= LFS_FILE_MAX
     // 为零时默认为LFS_FILE_MAX或存储在磁盘上的file_max
     pub file_max: LfsSize,
 
     // 自定义属性大小的可选上限（字节）。较大的属性大小没有缺点，但必须 <= LFS_ATTR_MAX
     // 为零时默认为LFS_ATTR_MAX或存储在磁盘上的attr_max
     pub attr_max: LfsSize,
 
     // 为元数据对给定的总空间的可选上限（字节）。在有大块的设备上（例如128kB）
     // 将此设置为较小的大小（2-8kB）可帮助限制元数据压缩时间。必须 <= block_size
     // 为零时默认为block_size
     pub metadata_max: LfsSize,
 
     // 内联文件的可选上限（字节）。内联文件存在于元数据中，减少存储需求
     // 但可能受限以提高元数据相关性能。必须 <= cache_size，<= attr_max，<= block_size/8
     // 为零时默认为可能的最大inline_max。设置为-1以禁用内联文件
     pub inline_max: LfsSize,
 
     // 写入时使用的磁盘版本，格式为16位主版本 + 16位次版本
     // 这限制了元数据对较旧次版本支持的内容。注意某些功能将丢失
     // 为零时默认为最新的次版本
     pub disk_version: u32,
 }
 
 /// 文件信息结构
 #[derive(Debug, Clone)]
 pub struct LfsInfo {
     // 文件类型，LFS_TYPE_REG或LFS_TYPE_DIR
     pub type_: u8,
 
     // 文件大小，仅对REG文件有效。限制为32位
     pub size: LfsSize,
 
     // 文件名，存储为以null结尾的字符串。限制为LFS_NAME_MAX+1
     // 可通过重新定义LFS_NAME_MAX减少RAM。LFS_NAME_MAX存储在超级块中
     // 其他littlefs驱动程序必须遵守
     pub name: [u8; LFS_NAME_MAX+1],
 }
 
 /// 文件系统信息结构
 #[derive(Debug, Clone)]
 pub struct LfsFsinfo {
     // 磁盘版本
     pub disk_version: u32,
 
     // 逻辑块大小（字节）
     pub block_size: LfsSize,
 
     // 文件系统中的逻辑块数量
     pub block_count: LfsSize,
 
     // 文件名长度的上限（字节）
     pub name_max: LfsSize,
 
     // 文件大小的上限（字节）
     pub file_max: LfsSize,
 
     // 自定义属性大小的上限（字节）
     pub attr_max: LfsSize,
 }
 
 /// 自定义属性结构，用于描述在文件写入期间原子提交的自定义属性
 #[derive(Debug)]
 pub struct LfsAttr {
     // 属性的8位类型，由用户提供并用于识别属性
     pub type_: u8,
 
     // 指向包含属性的缓冲区的指针
     pub buffer: *mut c_void,
 
     // 属性大小（字节），限制为LFS_ATTR_MAX
     pub size: LfsSize,
 }
 
 /// lfs_file_opencfg期间提供的可选配置
 #[derive(Debug)]
 pub struct LfsFileConfig {
     // 可选的静态分配文件缓冲区。必须为cache_size大小
     // 默认使用lfs_malloc分配此缓冲区
     pub buffer: *mut c_void,
 
     // 与文件相关的自定义属性的可选列表。如果文件以读取访问权限打开
     // 这些属性将在打开调用期间从磁盘读取。如果文件以写入访问权限打开
     // 属性将在每次文件同步或关闭时写入磁盘。此写入与文件内容的更新原子发生
     //
     // 自定义属性由8位类型唯一标识，并限制为LFS_ATTR_MAX字节
     // 读取时，如果存储的属性小于缓冲区，则用零填充
     // 如果存储的属性更大，则将静默截断。如果找不到属性，则隐式创建
     pub attrs: *mut LfsAttr,
 
     // 列表中的自定义属性数量
     pub attr_count: LfsSize,
 }
 
 /// 内部littlefs数据结构
 
 /// 缓存结构
 #[derive(Debug, Clone)]
 pub struct LfsCache {
     pub block: LfsBlock,
     pub off: LfsOff,
     pub size: LfsSize,
     pub buffer: *mut u8,
 }
 
 /// 元数据目录结构
 #[derive(Debug, Clone)]
 pub struct LfsMdir {
     pub pair: [LfsBlock; 2],
     pub rev: u32,
     pub off: LfsOff,
     pub etag: u32,
     pub count: u16,
     pub erased: bool,
     pub split: bool,
     pub tail: [LfsBlock; 2],
 }
 
 /// littlefs目录类型
 #[derive(Debug)]
 pub struct LfsDir {
     pub next: *mut LfsDir,
     pub id: u16,
     pub type_: u8,
     pub m: LfsMdir,
     pub pos: LfsOff,
     pub head: [LfsBlock; 2],
 }
 
 /// littlefs文件类型
 #[derive(Debug)]
 pub struct LfsFile {
     pub next: *mut LfsFile,
     pub id: u16,
     pub type_: u8,
     pub m: LfsMdir,
     pub ctz: LfsCtz,
     pub flags: u32,
     pub pos: LfsOff,
     pub block: LfsBlock,
     pub off: LfsOff,
     pub cache: LfsCache,
     pub cfg: *const LfsFileConfig,
 }
 
 /// CTZ（计数尾随零）结构
 #[derive(Debug, Clone)]
 pub struct LfsCtz {
     pub head: LfsBlock,
     pub size: LfsSize,
 }
 
 /// 超级块结构
 #[derive(Debug, Clone)]
 pub struct LfsSuperblock {
     pub version: u32,
     pub block_size: LfsSize,
     pub block_count: LfsSize,
     pub name_max: LfsSize,
     pub file_max: LfsSize,
     pub attr_max: LfsSize,
 }
 
 /// 全局状态结构
 #[derive(Debug, Clone)]
 pub struct LfsGstate {
     pub tag: u32,
     pub pair: [LfsBlock; 2],
 }
 
 /// 挂载列表结构
 #[derive(Debug)]
 pub struct LfsMlist {
     pub next: *mut LfsMlist,
     pub id: u16,
     pub type_: u8,
     pub m: LfsMdir,
 }
 
 /// 前瞻结构
 #[derive(Debug)]
 pub struct LfsLookahead {
     pub start: LfsBlock,
     pub size: LfsBlock,
     pub next: LfsBlock,
     pub ckpoint: LfsBlock,
     pub buffer: *mut u8,
 }
 
 /// LittleFS文件系统类型
 #[derive(Debug)]
 pub struct Lfs {
     pub rcache: LfsCache,
     pub pcache: LfsCache,
     pub root: [LfsBlock; 2],
     pub mlist: *mut LfsMlist,
     pub seed: u32,
     pub gstate: LfsGstate,
     pub gdisk: LfsGstate,
     pub gdelta: LfsGstate,
     pub lookahead: LfsLookahead,
     pub cfg: *const LfsConfig,
     pub block_count: LfsSize,
     pub name_max: LfsSize,
     pub file_max: LfsSize,
     pub attr_max: LfsSize,
     pub inline_max: LfsSize,
     pub lfs1: *mut c_void, // 可选的LFS1兼容性
 }
 
 // 注意：这里省略了函数声明，因为Rust中声明和实现不必分开
 // 这些函数将在后续任务中实现
 
 /// 特殊块值定义
 pub const LFS_BLOCK_NULL: LfsBlock = !0;   // 相当于C中的(lfs_block_t)-1
 pub const LFS_BLOCK_INLINE: LfsBlock = !1; // 相当于C中的(lfs_block_t)-2
 
 /// 操作结果类型
 #[repr(i32)]
 #[derive(Debug, PartialEq, Eq, Clone, Copy)]
 pub enum LfsOk {
     Relocated = 1,
     Dropped   = 2,
     Orphaned  = 3,
 }
 
 /// 比较结果类型
 #[repr(i32)]
 #[derive(Debug, PartialEq, Eq, Clone, Copy)]
 pub enum LfsCmp {
     Eq = 0,
     Lt = 1,
     Gt = 2,
 }
 
 /// 标签类型定义
 pub type LfsTag = u32;
 pub type LfsStag = i32;
 
 /// 元数据属性结构
 #[derive(Debug, Clone)]
 pub struct LfsMattr {
     pub tag: LfsTag,
     pub buffer: *const c_void,
 }
 
 /// 磁盘偏移结构
 #[derive(Debug, Clone)]
 pub struct LfsDiskoff {
     pub block: LfsBlock,
     pub off: LfsOff,
 }
 
 /// 文件CRC结构
 #[derive(Debug, Clone)]
 pub struct LfsFcrc {
     pub size: LfsSize,
     pub crc: u32,
 }
 
 /// 目录遍历深度常量
 pub const LFS_DIR_TRAVERSE_DEPTH: usize = 3;
 
 /// 目录遍历结构
 #[derive(Debug)]
 pub struct LfsDirTraverse {
     pub dir: *const LfsMdir,
     pub off: LfsOff,
     pub ptag: LfsTag,
     pub attrs: *const LfsMattr,
     pub attrcount: i32,
 
     pub tmask: LfsTag,
     pub ttag: LfsTag,
     pub begin: u16,
     pub end: u16,
     pub diff: i16,
 
     pub cb: Option<unsafe extern "C" fn(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32>,
     pub data: *mut c_void,
 
     pub tag: LfsTag,
     pub buffer: *const c_void,
     pub disk: LfsDiskoff,
 }
 
 /// 目录查找匹配结构
 #[derive(Debug)]
 pub struct LfsDirFindMatch {
     pub lfs: *mut Lfs,
     pub name: *const c_void,
     pub size: LfsSize,
 }
 
 /// 提交结构
 #[derive(Debug)]
 pub struct LfsCommit {
     pub block: LfsBlock,
     pub off: LfsOff,
     pub ptag: LfsTag,
     pub crc: u32,
 
     pub begin: LfsOff,
     pub end: LfsOff,
 }
 
 /// 目录提交结构
 #[derive(Debug)]
 pub struct LfsDirCommitCommit {
     pub lfs: *mut Lfs,
     pub commit: *mut LfsCommit,
 }
 
 /// 龟兔算法结构，用于检测循环
 #[derive(Debug)]
 pub struct LfsTortoise {
     pub pair: [LfsBlock; 2],
     pub i: LfsSize,
     pub period: LfsSize,
 }
 
 /// 文件系统父级匹配结构
 #[derive(Debug)]
 pub struct LfsFsParentMatch {
     pub lfs: *mut Lfs,
     pub pair: [LfsBlock; 2],
 }
 