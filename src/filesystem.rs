/*
 * The little filesystem
 *
 * Copyright (c) 2022, The littlefs authors.
 * Copyright (c) 2017, Arm Limited. All rights reserved.
 * SPDX-License-Identifier: BSD-3-Clause
 */

 use crate::lfs_util;
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
 
#[inline]
pub(crate) fn lfs_cache_drop(_lfs: &Lfs, rcache: &mut LfsCache) {
    // do not zero, cheaper if cache is readonly or only going to be
    // written with identical data (during relocates)
    rcache.block = LFS_BLOCK_NULL;
}

//  lfs_cache_zero
#[inline]
pub unsafe fn lfs_cache_zero(lfs: *mut Lfs, pcache: *mut LfsCache) {
    // zero to avoid information leak
    let cache_size = (*lfs).cfg.as_ref().unwrap().cache_size as usize;
    let pcache_buffer = (*pcache).buffer;
    core::ptr::write_bytes(pcache_buffer, 0xff, cache_size);
    (*pcache).block = LFS_BLOCK_NULL;
}

//  lfs_bd_read
pub(crate) fn lfs_bd_read(
    lfs: &mut Lfs,
    pcache: Option<&LfsCache>,
    rcache: Option<&mut LfsCache>,
    hint: LfsSize,
    block: LfsBlock,
    off: LfsOff,
    buffer: *mut c_void,
    size: LfsSize,
) -> Result<(), LfsError> {
    let mut data = buffer as *mut u8;
    let mut off = off;
    let mut size = size;

    // Check if we read past block or block is invalid
    if off + size > lfs.cfg.block_size 
        || (lfs.block_count != 0 && block >= lfs.block_count) 
    {
        return Err(LfsError::Corrupt);
    }

    while size > 0 {
        let mut diff = size;

        // Check pcache first if exists
        if let Some(pcache) = pcache {
            if block == pcache.block && off < pcache.off + pcache.size {
                if off >= pcache.off {
                    // Data is in pcache
                    let cache_off = (off - pcache.off) as usize;
                    let copy_size = (pcache.size as usize - cache_off).min(diff as usize);
                    
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            pcache.buffer.add(cache_off),
                            data,
                            copy_size,
                        );
                    }
                    
                    data = unsafe { data.add(copy_size) };
                    off += copy_size as LfsOff;
                    size -= copy_size as LfsSize;
                    continue;
                }

                // pcache has priority for preceding data
                diff = diff.min((pcache.off - off) as LfsSize);
            }
        }

        // Then check rcache if exists
        if let Some(rcache) = rcache {
            if block == rcache.block && off < rcache.off + rcache.size {
                if off >= rcache.off {
                    // Data is in rcache
                    let cache_off = (off - rcache.off) as usize;
                    let copy_size = (rcache.size as usize - cache_off).min(diff as usize);
                    
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            rcache.buffer.add(cache_off),
                            data,
                            copy_size,
                        );
                    }
                    
                    data = unsafe { data.add(copy_size) };
                    off += copy_size as LfsOff;
                    size -= copy_size as LfsSize;
                    continue;
                }

                // rcache has priority for preceding data
                diff = diff.min((rcache.off - off) as LfsSize);
            }
        }

        // Try direct read if conditions are met
        if size >= hint 
            && off % lfs.cfg.read_size == 0 
            && size >= lfs.cfg.read_size 
        {
            let aligned_size = lfs_util::lfs_aligndown(diff, lfs.cfg.read_size);
            
            let err = unsafe {
                (lfs.cfg.read.unwrap())(
                    lfs.cfg,
                    block,
                    off,
                    data as *mut c_void,
                    aligned_size,
                )
            };
            
            if err != 0 {
                return Err(LfsError::from(err));
            }
            
            data = unsafe { data.add(aligned_size as usize) };
            off += aligned_size;
            size -= aligned_size;
            continue;
        }

        // Load data into rcache
        debug_assert!(lfs.block_count == 0 || block < lfs.block_count);
        
        if let Some(rcache) = rcache {
            rcache.block = block;
            rcache.off = lfs_util::lfs_aligndown(off, lfs.cfg.read_size);
            
            let aligned_end = lfs_util::lfs_alignup(off + hint, lfs.cfg.read_size);
            let max_size = lfs.cfg.block_size - rcache.off;
            
            rcache.size = aligned_end
                .min(max_size)
                .min(lfs.cfg.cache_size)
                - rcache.off;
            
            let err = unsafe {
                (lfs.cfg.read.unwrap())(
                    lfs.cfg,
                    rcache.block,
                    rcache.off,
                    rcache.buffer as *mut c_void,
                    rcache.size,
                )
            };
            
            if err != 0 {
                return Err(LfsError::from(err));
            }
        } else {
            return Err(LfsError::Corrupt);
        }
    }

    Ok(())
}

//  lfs_bd_cmp
#[no_mangle]
pub unsafe extern "C" fn lfs_bd_cmp(
    lfs: *mut Lfs,
    pcache: *const LfsCache,
    rcache: *const LfsCache,
    hint: LfsSize,
    block: LfsBlock,
    off: LfsOff,
    buffer: *const c_void,
    size: LfsSize,
) -> i32 {
    let data = buffer as *const u8;
    let mut diff = 0;
    let mut i: LfsOff = 0;

    while i < size {
        let mut dat = [0u8; 8];
        diff = (size - i).min(8) as LfsSize;

        let err = lfs_bd_read(
            lfs,
            pcache,
            rcache,
            hint.wrapping_sub(i as LfsSize),
            block,
            off.wrapping_add(i),
            dat.as_mut_ptr() as *mut c_void,
            diff,
        );
        if err != 0 {
            return err;
        }

        let current_data = data.add(i as usize);
        let mut res = 0;
        for j in 0..diff as usize {
            let a = *current_data.add(j);
            let b = dat[j];
            if a != b {
                res = a as i32 - b as i32;
                break;
            }
        }

        if res != 0 {
            return if res < 0 {
                LfsCmp::Lt as i32
            } else {
                LfsCmp::Gt as i32
            };
        }

        i += diff as LfsOff;
    }

    LfsCmp::Eq as i32
}

//  lfs_bd_crc
pub fn lfs_bd_crc(
    lfs: &mut Lfs,
    pcache: &LfsCache,
    rcache: &LfsCache,
    hint: LfsSize,
    block: LfsBlock,
    off: LfsOff,
    size: LfsSize,
    crc: &mut u32,
) -> i32 {
    let mut i = 0;
    while i < size {
        let mut dat = [0u8; 8];
        let diff = (size - i).min(dat.len() as LfsSize);
        
        let err = lfs_bd_read(
            lfs,
            pcache,
            rcache,
            hint.wrapping_sub(i),
            block,
            off.wrapping_add(i),
            dat.as_mut_ptr() as *mut c_void,
            diff,
        );
        
        if err != 0 {
            return err;
        }
        
        *crc = lfs_crc(*crc, &dat[0..diff as usize]);
        i += diff;
    }
    
    0
}

//  lfs_bd_flush
pub(crate) fn lfs_bd_flush(
    lfs: &mut Lfs,
    pcache: &mut LfsCache,
    rcache: &mut LfsCache,
    validate: bool,
) -> i32 {
    if pcache.block != LFS_BLOCK_NULL && pcache.block != LFS_BLOCK_INLINE {
        assert!(pcache.block < lfs.block_count);
        let cfg = unsafe { &*lfs.cfg };
        let diff = lfs_util::lfs_alignup(pcache.size, cfg.prog_size);
        let err = unsafe {
            (cfg.prog.unwrap())(
                lfs.cfg,
                pcache.block,
                pcache.off,
                pcache.buffer as *const c_void,
                diff,
            )
        };
        assert!(err <= 0);
        if err != 0 {
            return err;
        }

        if validate {
            lfs_cache_drop(lfs, rcache);
            let res = lfs_bd_cmp(
                lfs,
                None,
                rcache,
                diff,
                pcache.block,
                pcache.off,
                pcache.buffer as *const c_void,
                diff,
            );
            if res < 0 {
                return res;
            }
            if res != LfsCmp::Eq as i32 {
                return LfsError::Corrupt as i32;
            }
        }

        lfs_cache_zero(lfs, pcache);
    }

    0
}

//  lfs_bd_sync
pub unsafe fn lfs_bd_sync(
    lfs: &mut Lfs,
    pcache: &mut LfsCache,
    rcache: &mut LfsCache,
    validate: bool,
) -> i32 {
    lfs_cache_drop(lfs, rcache);

    let err = lfs_bd_flush(lfs, pcache, rcache, validate);
    if err != 0 {
        return err;
    }

    let cfg = lfs.cfg;
    let err = ((*cfg).sync.unwrap())(cfg);
    assert!(err <= 0, "sync returned positive error code");
    err
}

//  lfs_bd_prog
pub unsafe fn lfs_bd_prog(
    lfs: *mut Lfs,
    pcache: *mut LfsCache,
    rcache: *mut LfsCache,
    validate: bool,
    block: LfsBlock,
    off: LfsOff,
    buffer: *const c_void,
    size: LfsSize,
) -> i32 {
    let mut data = buffer as *const u8;
    let mut off = off;
    let mut size = size;
    let cfg = &*((*lfs).cfg);

    // 参数验证断言
    assert!(block == LFS_BLOCK_INLINE || block < (*lfs).block_count);
    assert!(off.checked_add(size).unwrap() <= cfg.block_size);

    while size > 0 {
        let pcache_block = (*pcache).block;
        let pcache_off = (*pcache).off;
        let cache_size = cfg.cache_size;

        // 检查是否在pcache范围内
        if block == pcache_block 
            && off >= pcache_off 
            && off < pcache_off + cache_size 
        {
            // 计算可复制数据量
            let diff = core::cmp::min(
                size, 
                cache_size - (off - pcache_off)
            );

            // 复制数据到缓存
            core::ptr::copy_nonoverlapping(
                data,
                (*pcache).buffer.add((off - pcache_off) as usize),
                diff as usize
            );

            // 更新指针和计数器
            data = data.add(diff as usize);
            off += diff;
            size -= diff;

            // 更新缓存大小并检查是否满
            (*pcache).size = core::cmp::max((*pcache).size, off - pcache_off);
            if (*pcache).size == cache_size {
                // 触发缓存刷新
                let err = lfs_bd_flush(lfs, pcache, rcache, validate);
                if err != 0 {
                    return err;
                }
            }

            continue;
        }

        // 验证pcache必须已刷新
        assert!((*pcache).block == LFS_BLOCK_NULL);

        // 初始化新的pcache条目
        (*pcache).block = block;
        (*pcache).off = lfs_util::lfs_aligndown(off, cfg.prog_size);
        (*pcache).size = 0;
    }

    0
}

//  lfs_bd_erase
pub unsafe extern "C" fn lfs_bd_erase(lfs: *mut Lfs, block: LfsBlock) -> i32 {
    debug_assert!(block < (*lfs).block_count);
    let cfg = (*lfs).cfg;
    debug_assert!((*cfg).erase.is_some(), "erase function must be provided");
    let err = ((*cfg).erase.unwrap())(cfg, block);
    debug_assert!(err <= 0);
    err
}

//  lfs_path_namelen
#[inline]
pub unsafe extern "C" fn lfs_path_namelen(path: *const core::ffi::c_char) -> LfsSize {
    let cstr = core::ffi::CStr::from_ptr(path);
    let bytes = cstr.to_bytes();
    bytes.iter().position(|&c| c == b'/').unwrap_or(bytes.len()) as LfsSize
}

//  lfs_path_islast
#[inline]
pub(crate) unsafe fn lfs_path_islast(path: *const c_char) -> bool {
    let namelen = lfs_path_namelen(path);
    let path_str = CStr::from_ptr(path);
    let bytes = path_str.to_bytes();
    let start = namelen as usize;
    let skip = bytes[start..].iter()
        .take_while(|&&c| c == b'/')
        .count();
    bytes.get(start + skip).copied() == Some(0)
}

//  lfs_path_isdir
#[inline]
pub unsafe fn lfs_path_isdir(path: *const u8) -> bool {
    *path.add(lfs_path_namelen(path)) != b'\0'
}

//  lfs_pair_swap
#[inline]
pub fn lfs_pair_swap(pair: &mut [LfsBlock; 2]) {
    let t = pair[0];
    pair[0] = pair[1];
    pair[1] = t;
}

//  lfs_pair_isnull
/// Check if a block pair is null
#[inline]
pub fn lfs_pair_isnull(pair: [LfsBlock; 2]) -> bool {
    pair[0] == LFS_BLOCK_NULL || pair[1] == LFS_BLOCK_NULL
}

//  lfs_pair_cmp
#[inline]
pub fn lfs_pair_cmp(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> i32 {
    !(paira[0] == pairb[0] || paira[1] == pairb[1] 
        || paira[0] == pairb[1] || paira[1] == pairb[0]) as i32
}

//  lfs_pair_issync
#[inline]
pub fn lfs_pair_issync(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> bool {
    (paira[0] == pairb[0] && paira[1] == pairb[1]) ||
    (paira[0] == pairb[1] && paira[1] == pairb[0])
}

//  lfs_pair_fromle32
#[inline]
pub(crate) fn lfs_pair_fromle32(pair: &mut [LfsBlock; 2]) {
    pair[0] = lfs_util::lfs_fromle32(pair[0]);
    pair[1] = lfs_util::lfs_fromle32(pair[1]);
}

//  lfs_pair_tole32
#[inline]
pub(crate) fn lfs_pair_tole32(pair: &mut [LfsBlock; 2]) {
    pair[0] = pair[0].to_le();
    pair[1] = pair[1].to_le();
}

//  lfs_tag_isvalid
#[inline]
pub fn lfs_tag_isvalid(tag: LfsTag) -> bool {
    (tag & 0x80000000) == 0
}

//  lfs_tag_isdelete
#[inline]
pub fn lfs_tag_isdelete(tag: LfsTag) -> bool {
    ((tag << 22) as i32 >> 22) == -1
}

//  lfs_tag_type1
#[inline]
pub fn lfs_tag_type1(tag: LfsTag) -> u16 {
    ((tag & 0x70000000) >> 20) as u16
}

//  lfs_tag_type2
#[inline]
pub fn lfs_tag_type2(tag: LfsTag) -> u16 {
    ((tag & 0x78000000) >> 20) as u16
}

//  lfs_tag_type3
#[inline]
pub fn lfs_tag_type3(tag: LfsTag) -> u16 {
    ((tag & 0x7ff00000) >> 20) as u16
}

//  lfs_tag_chunk
pub fn lfs_tag_chunk(tag: LfsTag) -> u8 {
    ((tag & 0x0ff00000) >> 20) as u8
}

//  lfs_tag_splice
#[inline]
pub fn lfs_tag_splice(tag: LfsTag) -> i8 {
    lfs_tag_chunk(tag) as i8
}

//  lfs_tag_id
#[inline]
pub fn lfs_tag_id(tag: LfsTag) -> u16 {
    ((tag & 0x000ffc00) >> 10) as u16
}

//  lfs_tag_size
#[inline]
pub fn lfs_tag_size(tag: LfsTag) -> LfsSize {
    tag & 0x000003ff
}

//  lfs_tag_dsize
#[inline]
pub(crate) fn lfs_tag_dsize(tag: LfsTag) -> LfsSize {
    (core::mem::size_of::<LfsTag>() as LfsSize)
        .wrapping_add(lfs_tag_size(tag.wrapping_add(lfs_tag_isdelete(tag))))
}

//  lfs_gstate_xor
#[inline]
pub unsafe extern "C" fn lfs_gstate_xor(a: *mut LfsGstate, b: *const LfsGstate) {
    (*a).tag ^= (*b).tag;
    (*a).pair[0] ^= (*b).pair[0];
    (*a).pair[1] ^= (*b).pair[1];
}

//  lfs_gstate_iszero
#[inline]
pub(crate) fn lfs_gstate_iszero(a: &LfsGstate) -> bool {
    a.tag == 0 && a.pair[0] == 0 && a.pair[1] == 0
}

//  lfs_gstate_hasorphans
#[inline]
pub fn lfs_gstate_hasorphans(a: &LfsGstate) -> bool {
    lfs_tag_size(a.tag) != 0
}

//  lfs_gstate_getorphans
#[inline]
pub(crate) fn lfs_gstate_getorphans(a: &LfsGstate) -> u8 {
    (lfs_tag_size(a.tag) & 0x1ff) as u8
}

//  lfs_gstate_hasmove
#[inline]
pub fn lfs_gstate_hasmove(a: &LfsGstate) -> bool {
    lfs_tag_type1(a.tag)
}

//  lfs_gstate_needssuperblock
#[inline]
pub fn lfs_gstate_needssuperblock(a: &LfsGstate) -> bool {
    (lfs_tag_size(a.tag) >> 9) != 0
}

//  lfs_gstate_hasmovehere
#[inline]
pub fn lfs_gstate_hasmovehere(a: &LfsGstate, pair: &[LfsBlock; 2]) -> bool {
    lfs_tag_type1(a.tag) && lfs_pair_cmp(&a.pair, pair) == 0
}

//  lfs_gstate_fromle32
pub fn lfs_gstate_fromle32(a: &mut LfsGstate) {
    a.tag = lfs_util::lfs_fromle32(a.tag);
    a.pair[0] = lfs_util::lfs_fromle32(a.pair[0]);
    a.pair[1] = lfs_util::lfs_fromle32(a.pair[1]);
}

//  lfs_gstate_tole32
#[inline]
pub fn lfs_gstate_tole32(gstate: &mut LfsGstate) {
    gstate.tag = lfs_util::lfs_tole32(gstate.tag);
    gstate.pair[0] = lfs_util::lfs_tole32(gstate.pair[0]);
    gstate.pair[1] = lfs_util::lfs_tole32(gstate.pair[1]);
}

//  lfs_fcrc_fromle32
pub fn lfs_fcrc_fromle32(fcrc: &mut LfsFcrc) {
    fcrc.size = u32::from_le(fcrc.size);
    fcrc.crc = u32::from_le(fcrc.crc);
}

//  lfs_fcrc_tole32
pub fn lfs_fcrc_tole32(fcrc: &mut LfsFcrc) {
    fcrc.size = lfs_util::lfs_tole32(fcrc.size);
    fcrc.crc = lfs_util::lfs_tole32(fcrc.crc);
}

//  lfs_ctz_fromle32
pub fn lfs_ctz_fromle32(ctz: &mut LfsCtz) {
    ctz.head = crate::lfs_util::lfs_fromle32(ctz.head);
    ctz.size = crate::lfs_util::lfs_fromle32(ctz.size);
}

//  lfs_ctz_tole32
pub fn lfs_ctz_tole32(ctz: &mut LfsCtz) {
    ctz.head = ctz.head.to_le();
    ctz.size = ctz.size.to_le();
}

//  lfs_superblock_fromle32
#[inline]
pub fn lfs_superblock_fromle32(superblock: &mut LfsSuperblock) {
    superblock.version = lfs_util::lfs_fromle32(superblock.version);
    superblock.block_size = lfs_util::lfs_fromle32(superblock.block_size);
    superblock.block_count = lfs_util::lfs_fromle32(superblock.block_count);
    superblock.name_max = lfs_util::lfs_fromle32(superblock.name_max);
    superblock.file_max = lfs_util::lfs_fromle32(superblock.file_max);
    superblock.attr_max = lfs_util::lfs_fromle32(superblock.attr_max);
}

//  lfs_superblock_tole32
#[inline]
pub fn lfs_superblock_tole32(superblock: &mut LfsSuperblock) {
    superblock.version = lfs_util::lfs_tole32(superblock.version);
    superblock.block_size = lfs_util::lfs_tole32(superblock.block_size);
    superblock.block_count = lfs_util::lfs_tole32(superblock.block_count);
    superblock.name_max = lfs_util::lfs_tole32(superblock.name_max);
    superblock.file_max = lfs_util::lfs_tole32(superblock.file_max);
    superblock.attr_max = lfs_util::lfs_tole32(superblock.attr_max);
}

//  lfs_mlist_isopen
pub unsafe fn lfs_mlist_isopen(head: *mut LfsMlist, node: *mut LfsMlist) -> bool {
    let mut p = &mut head;
    while !(*p).is_null() {
        if *p == node {
            return true;
        }
        p = &mut (*p).as_mut().unwrap().next;
    }
    false
}

//  lfs_mlist_remove
pub unsafe extern "C" fn lfs_mlist_remove(lfs: *mut Lfs, mlist: *mut LfsMlist) {
    let mut p = &mut (*lfs).mlist;
    while !(*p).is_null() {
        if *p == mlist {
            *p = (**p).next;
            break;
        }
        p = &mut (**p).next;
    }
}

//  lfs_mlist_append
pub unsafe fn lfs_mlist_append(lfs: &mut Lfs, mlist: *mut LfsMlist) {
    // 安全说明：通过原始指针操作链表是必要的FFI兼容需求
    // 调用方需保证指针有效性
    unsafe {
        (*mlist).next = lfs.mlist;
        lfs.mlist = mlist;
    }
}

//  lfs_fs_disk_version
pub fn lfs_fs_disk_version(lfs: *const Lfs) -> u32 {
    #[cfg(feature = "multiversion")]
    {
        unsafe {
            let cfg = &*(*lfs).cfg;
            if cfg.disk_version != 0 {
                cfg.disk_version
            } else {
                LFS_DISK_VERSION
            }
        }
    }
    #[cfg(not(feature = "multiversion"))]
    {
        let _ = lfs;
        LFS_DISK_VERSION
    }
}

//  lfs_fs_disk_version_major
#[no_mangle]
pub unsafe extern "C" fn lfs_fs_disk_version_major(lfs: *mut Lfs) -> u16 {
    (0xffff & (lfs_fs_disk_version(lfs) >> 16)) as u16
}

//  lfs_fs_disk_version_minor
pub unsafe extern "C" fn lfs_fs_disk_version_minor(lfs: *mut Lfs) -> u16 {
    (lfs_fs_disk_version(lfs) >> 0) as u16 & 0xffff
}

//  lfs_alloc_ckpoint
pub(crate) fn lfs_alloc_ckpoint(lfs: &mut Lfs) {
    lfs.lookahead.ckpoint = lfs.block_count;
}

//  lfs_alloc_drop
pub fn lfs_alloc_drop(lfs: &mut Lfs) {
    lfs.lookahead.size = 0;
    lfs.lookahead.next = 0;
    lfs_alloc_ckpoint(lfs);
}

//  lfs_alloc_lookahead
pub unsafe extern "C" fn lfs_alloc_lookahead(p: *mut c_void, block: LfsBlock) -> i32 {
    let lfs = p as *mut Lfs;
    let off = ((block.wrapping_sub((*lfs).lookahead.start)).wrapping_add((*lfs).block_count)) % (*lfs).block_count;
    
    if off < (*lfs).lookahead.size {
        let byte = (*lfs).lookahead.buffer.add((off / 8) as usize);
        *byte |= 1u8 << (off % 8);
    }
    
    0
}

//  lfs_alloc_scan
pub fn lfs_alloc_scan(lfs: &mut Lfs) -> i32 {
    // move lookahead buffer to the first unused block
    // note we limit the lookahead buffer to at most the amount of blocks
    // checkpointed, this prevents the math in lfs_alloc from underflowing
    lfs.lookahead.start = (lfs.lookahead.start + lfs.lookahead.next) 
        % lfs.block_count;
    lfs.lookahead.next = 0;
    let lookahead_size = unsafe { (*lfs.cfg).lookahead_size };
    lfs.lookahead.size = core::cmp::min(
        8 * lookahead_size,
        lfs.lookahead.ckpoint,
    );

    // find mask of free blocks from tree
    unsafe {
        core::ptr::write_bytes(
            lfs.lookahead.buffer,
            0,
            lookahead_size as usize,
        );
    }
    let err = unsafe {
        lfs_fs_traverse_(
            lfs as *mut _ as *mut c_void,
            Some(lfs_alloc_lookahead),
            lfs as *mut _ as *mut c_void,
            true,
        )
    };
    if err != 0 {
        lfs_alloc_drop(lfs);
        return err;
    }

    0
}

//  lfs_alloc
pub fn lfs_alloc(lfs: &mut Lfs, block: *mut LfsBlock) -> Result<(), LfsError> {
    loop {
        // Scan lookahead buffer for free blocks
        while lfs.lookahead.next < lfs.lookahead.size {
            let byte_index = lfs.lookahead.next / 8;
            let bit = lfs.lookahead.next % 8;
            let mask = 1u8 << bit;

            // Check if current bit is unset (free block)
            unsafe {
                if (*lfs.lookahead.buffer.add(byte_index as usize) & mask) == 0 {
                    // Found free block, calculate block number
                    *block = (lfs.lookahead.start + lfs.lookahead.next) % lfs.block_count;

                    // Find next non-free block or end of buffer
                    loop {
                        lfs.lookahead.next += 1;
                        lfs.lookahead.ckpoint = lfs.lookahead.ckpoint.wrapping_sub(1);

                        if lfs.lookahead.next >= lfs.lookahead.size {
                            return Ok(());
                        }

                        let next_byte = lfs.lookahead.next / 8;
                        let next_bit = lfs.lookahead.next % 8;
                        if (*lfs.lookahead.buffer.add(next_byte as usize) & (1u8 << next_bit)) == 0 {
                            return Ok(());
                        }
                    }
                }
            }

            // Continue searching
            lfs.lookahead.next += 1;
            lfs.lookahead.ckpoint = lfs.lookahead.ckpoint.wrapping_sub(1);
        }

        // Check storage exhaustion
        if lfs.lookahead.ckpoint <= 0 {
            return Err(LfsError::NoSpc);
        }

        // Scan next lookahead window
        lfs_alloc_scan(lfs)?;
    }
}

//  lfs_dir_getslice
pub(crate) static mut LFS_TYPE_SPLICE: u32 = 0x400;

#[no_mangle]
pub unsafe extern "C" fn lfs_dir_getslice(
    lfs: &mut Lfs,
    dir: &LfsMdir,
    gmask: LfsTag,
    gtag: LfsTag,
    goff: LfsOff,
    gbuffer: *mut c_void,
    gsize: LfsSize,
) -> LfsStag {
    let mut off = dir.off;
    let mut ntag = dir.etag;
    let mut gdiff: LfsStag = 0;

    // Synthetic moves
    if lfs_gstate_hasmovehere(&lfs.gdisk, dir.pair) && lfs_tag_id(gmask) != 0 {
        if lfs_tag_id(lfs.gdisk.tag) == lfs_tag_id(gtag) {
            return LfsError::NoEnt as LfsStag;
        } else if lfs_tag_id(lfs.gdisk.tag) < lfs_tag_id(gtag) {
            gdiff -= LFS_MKTAG(0, 1, 0) as LfsStag;
        }
    }

    // Iterate over dir block backwards
    while off >= 4 + lfs_tag_dsize(ntag) {
        off -= lfs_tag_dsize(ntag);
        let tag = ntag;
        let mut read_ntag = 0u32;
        let err = lfs_bd_read(
            lfs,
            None,
            &mut lfs.rcache,
            4,
            dir.pair[0],
            off,
            &mut read_ntag as *mut _ as *mut c_void,
            4,
        );
        if err != 0 {
            return err as LfsStag;
        }
        ntag = (lfs_frombe32(read_ntag) ^ tag) & 0x7FFFFFFF;

        if lfs_tag_id(gmask) != 0
            && lfs_tag_type1(tag) == LFS_TYPE_SPLICE
            && lfs_tag_id(tag) <= lfs_tag_id(gtag.wrapping_sub(gdiff as u32))
        {
            if tag
                == (LFS_MKTAG(LFS_TYPE_CREATE, 0, 0)
                    | (LFS_MKTAG(0, 0x3FF, 0) & gtag.wrapping_sub(gdiff as u32)))
            {
                return LfsError::NoEnt as LfsStag;
            }

            gdiff += LFS_MKTAG(0, lfs_tag_splice(tag), 0) as LfsStag;
        }

        if (gmask & tag) == (gmask & gtag.wrapping_sub(gdiff as u32)) {
            if lfs_tag_isdelete(tag) {
                return LfsError::NoEnt as LfsStag;
            }

            let diff = lfs_min(lfs_tag_size(tag), gsize);
            let err = lfs_bd_read(
                lfs,
                None,
                &mut lfs.rcache,
                diff,
                dir.pair[0],
                off + 4 + goff,
                gbuffer,
                diff,
            );
            if err != 0 {
                return err as LfsStag;
            }

            let gbuffer_slice = core::slice::from_raw_parts_mut(gbuffer as *mut u8, gsize as usize);
            gbuffer_slice[diff as usize..].fill(0);

            return (tag as LfsStag).wrapping_add(gdiff);
        }
    }

    LfsError::NoEnt as LfsStag
}

//  lfs_dir_get
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_get(
    lfs: *mut Lfs,
    dir: *const LfsMdir,
    gmask: LfsTag,
    gtag: LfsTag,
    buffer: *mut c_void,
) -> LfsStag {
    lfs_dir_getslice(
        lfs,
        dir,
        gmask,
        gtag,
        0,
        buffer,
        (gtag & 0x00000fff) as LfsSize,
    )
}

//  lfs_dir_getread
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_getread(
    lfs: &mut Lfs,
    dir: &LfsMdir,
    pcache: Option<&LfsCache>,
    rcache: &mut LfsCache,
    hint: LfsSize,
    gmask: LfsTag,
    gtag: LfsTag,
    off: LfsOff,
    buffer: *mut c_void,
    size: LfsSize,
) -> i32 {
    let data = buffer as *mut u8;
    if off.checked_add(size).map_or(true, |sum| sum > lfs.cfg.block_size) {
        return LfsError::Corrupt as i32;
    }

    let mut data = data;
    let mut off = off;
    let mut size = size;

    while size > 0 {
        let mut diff = size;

        if let Some(pcache) = pcache {
            if pcache.block == LFS_BLOCK_INLINE && off < pcache.off + pcache.size {
                if off >= pcache.off {
                    // Data already in pcache
                    let cache_off = (off - pcache.off) as usize;
                    diff = diff.min(pcache.size - (off - pcache.off));
                    ptr::copy_nonoverlapping(
                        pcache.buffer.add(cache_off),
                        data,
                        diff as usize,
                    );
                    data = data.add(diff as usize);
                    off += diff;
                    size -= diff;
                    continue;
                }

                // pcache takes priority
                diff = diff.min(pcache.off - off);
            }
        }

        if rcache.block == LFS_BLOCK_INLINE && off < rcache.off + rcache.size {
            if off >= rcache.off {
                // Data already in rcache
                let cache_off = (off - rcache.off) as usize;
                diff = diff.min(rcache.size - (off - rcache.off));
                ptr::copy_nonoverlapping(
                    rcache.buffer.add(cache_off),
                    data,
                    diff as usize,
                );
                data = data.add(diff as usize);
                off += diff;
                size -= diff;
                continue;
            }

            // rcache takes priority
            diff = diff.min(rcache.off - off);
        }

        // Load to cache
        rcache.block = LFS_BLOCK_INLINE;
        rcache.off = lfs_util::lfs_aligndown(off, lfs.cfg.read_size);
        rcache.size = lfs_util::lfs_min(
            lfs_util::lfs_alignup(off + hint, lfs.cfg.read_size),
            lfs.cfg.cache_size,
        );

        let err = lfs_dir_getslice(
            lfs,
            dir,
            gmask,
            gtag,
            rcache.off,
            rcache.buffer as *mut c_void,
            rcache.size,
        );

        if err < 0 {
            return err;
        }
    }

    LfsError::Ok as i32
}

//  lfs_dir_traverse
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_traverse(
    lfs: *mut Lfs,
    dir: *const LfsMdir,
    mut off: LfsOff,
    mut ptag: LfsTag,
    mut attrs: *const LfsMattr,
    mut attrcount: i32,
    mut tmask: LfsTag,
    mut ttag: LfsTag,
    mut begin: u16,
    mut end: u16,
    mut diff: i16,
    mut cb: Option<unsafe extern "C" fn(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32>,
    mut data: *mut c_void,
) -> i32 {
    const LFS_DIR_TRAVERSE_DEPTH: usize = 3;
    let mut stack: [LfsDirTraverse; LFS_DIR_TRAVERSE_DEPTH - 1] = core::mem::zeroed();
    let mut sp: usize = 0;
    let mut res: i32 = 0;

    let mut tag: LfsTag = 0;
    let mut buffer: *const c_void = core::ptr::null();
    let mut disk = LfsDiskoff {
        block: 0,
        off: 0,
    };

    let mut popped = false;
    loop {
        if !popped {
            if (*dir).off > off + lfs_tag_dsize(ptag) {
                off += lfs_tag_dsize(ptag);
                tag = 0;
                let err = lfs_bd_read(
                    lfs,
                    core::ptr::null_mut(),
                    &mut (*lfs).rcache,
                    core::mem::size_of::<LfsTag>() as LfsSize,
                    (*dir).pair[0],
                    off,
                    &mut tag as *mut _ as *mut c_void,
                    core::mem::size_of::<LfsTag>() as LfsSize,
                );
                if err != 0 {
                    return err;
                }

                tag = (u32::from_be(tag) ^ ptag) | 0x80000000;
                disk.block = (*dir).pair[0];
                disk.off = off + core::mem::size_of::<LfsTag>() as LfsOff;
                buffer = &mut disk as *mut _ as *const c_void;
                ptag = tag;
            } else if attrcount > 0 {
                tag = (*attrs).tag;
                buffer = (*attrs).buffer;
                attrs = attrs.add(1);
                attrcount -= 1;
            } else {
                res = 0;
                popped = true;
                continue;
            }

            let mask = LFS_MKTAG(0x7ff, 0, 0);
            if (mask & tmask & tag) != (mask & tmask & ttag) {
                popped = false;
                continue;
            }

            if lfs_tag_id(tmask) != 0 {
                assert!(sp < LFS_DIR_TRAVERSE_DEPTH);
                stack[sp] = LfsDirTraverse {
                    dir,
                    off,
                    ptag,
                    attrs,
                    attrcount,
                    tmask,
                    ttag,
                    begin,
                    end,
                    diff,
                    cb,
                    data,
                    tag,
                    buffer,
                    disk,
                };
                sp += 1;

                tmask = 0;
                ttag = 0;
                begin = 0;
                end = 0;
                diff = 0;
                cb = Some(lfs_dir_traverse_filter);
                data = &mut stack[sp - 1].tag as *mut _ as *mut c_void;
                popped = false;
                continue;
            }
        }

        if lfs_tag_id(tmask) != 0 && !(lfs_tag_id(tag) >= begin && lfs_tag_id(tag) < end) {
            popped = false;
            continue;
        }

        match lfs_tag_type3(tag) {
            LfsType::FromNoop => {}
            LfsType::FromMove => {
                if cb == Some(lfs_dir_traverse_filter) {
                    popped = false;
                    continue;
                }

                stack[sp] = LfsDirTraverse {
                    dir,
                    off,
                    ptag,
                    attrs,
                    attrcount,
                    tmask,
                    ttag,
                    begin,
                    end,
                    diff,
                    cb,
                    data,
                    tag: LFS_MKTAG(LfsType::FromNoop as u32, 0, 0),
                    buffer: core::ptr::null(),
                    disk: LfsDiskoff { block: 0, off: 0 },
                };
                sp += 1;

                let fromid = lfs_tag_size(tag);
                let toid = lfs_tag_id(tag);
                dir = buffer as *const LfsMdir;
                off = 0;
                ptag = 0xffffffff;
                attrs = core::ptr::null();
                attrcount = 0;
                tmask = LFS_MKTAG(0x600, 0x3ff, 0);
                ttag = LFS_MKTAG(LfsType::Struct as u32, 0, 0);
                begin = fromid;
                end = fromid + 1;
                diff = toid.wrapping_sub(fromid) as i16 + diff;
            }
            LfsType::FromUserAttrs => {
                let a = buffer as *const LfsAttr;
                for i in 0..lfs_tag_size(tag) {
                    res = cb.expect("non-null callback")(
                        data,
                        LFS_MKTAG(
                            LfsType::UserAttr as u32 + (*a.add(i as usize)).type_ as u32,
                            lfs_tag_id(tag).wrapping_add(diff as u16),
                            (*a.add(i as usize)).size,
                        ),
                        (*a.add(i as usize)).buffer,
                    );
                    if res < 0 {
                        return res;
                    }
                    if res != 0 {
                        break;
                    }
                }
            }
            _ => {
                res = cb.expect("non-null callback")(
                    data,
                    tag + LFS_MKTAG(0, diff as u16, 0),
                    buffer,
                );
                if res < 0 {
                    return res;
                }
            }
        }

        if res != 0 {
            popped = true;
        }

        if sp > 0 {
            sp -= 1;
            dir = stack[sp].dir;
            off = stack[sp].off;
            ptag = stack[sp].ptag;
            attrs = stack[sp].attrs;
            attrcount = stack[sp].attrcount;
            tmask = stack[sp].tmask;
            ttag = stack[sp].ttag;
            begin = stack[sp].begin;
            end = stack[sp].end;
            diff = stack[sp].diff;
            cb = stack[sp].cb;
            data = stack[sp].data;
            tag = stack[sp].tag;
            buffer = stack[sp].buffer;
            disk = stack[sp].disk;
            popped = true;
        } else {
            return res;
        }
    }
}

// 辅助宏定义
const fn LFS_MKTAG(a: u32, b: u32, c: u32) -> u32 {
    (a << 20) | (b << 10) | c
}

const fn lfs_tag_id(tag: u32) -> u16 {
    ((tag >> 10) & 0x3ff) as u16
}

const fn lfs_tag_type3(tag: u32) -> LfsType {
    match (tag >> 20) & 0x7ff {
        0x000 => LfsType::FromNoop,
        0x101 => LfsType::FromMove,
        0x102 => LfsType::FromUserAttrs,
        _ => unsafe { core::mem::transmute((tag >> 20) as u16) },
    }
}

const fn lfs_tag_size(tag: u32) -> u16 {
    (tag & 0x3ff) as u16
}

const fn lfs_tag_dsize(tag: u32) -> LfsOff {
    (lfs_tag_size(tag) as LfsOff + sizeof::<LfsTag>() as LfsOff) & !0x1
}

const fn sizeof<T>() -> usize {
    core::mem::size_of::<T>()
}

//  lfs_dir_fetchmatch
pub(crate) static mut LFS_DIR_FETCHMATCH: unsafe extern "C" fn(
    lfs: *mut Lfs,
    dir: *mut LfsMdir,
    pair: [LfsBlock; 2],
    fmask: LfsTag,
    ftag: LfsTag,
    id: *mut u16,
    cb: Option<unsafe extern "C" fn(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32>,
    data: *mut c_void,
) -> LfsStag = {
    unsafe extern "C" fn implementation(
        lfs: *mut Lfs,
        dir: *mut LfsMdir,
        pair: [LfsBlock; 2],
        fmask: LfsTag,
        ftag: LfsTag,
        id: *mut u16,
        cb: Option<unsafe extern "C" fn(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32>,
        data: *mut c_void,
    ) -> LfsStag {
        let mut besttag: LfsStag = -1;

        // Validate block addresses
        if (*lfs).block_count != 0
            && (pair[0] >= (*lfs).block_count || pair[1] >= (*lfs).block_count)
        {
            return LfsError::Corrupt as LfsStag;
        }

        // Find most recent revision
        let mut revs = [0u32; 2];
        let mut r = 0;
        for i in 0..2 {
            let mut rev = 0u32;
            let err = lfs_bd_read(
                lfs,
                std::ptr::null_mut(),
                &mut (*lfs).rcache,
                std::mem::size_of::<u32>() as LfsSize,
                pair[i],
                0,
                &mut rev as *mut _ as *mut c_void,
                std::mem::size_of::<u32>() as LfsSize,
            );
            rev = lfs_fromle32(rev);
            revs[i] = rev;

            if err != 0 && err != LfsError::Corrupt as i32 {
                return err as LfsStag;
            }

            if err != LfsError::Corrupt as i32
                && lfs_scmp(revs[i], revs[(i + 1) % 2]) > 0
            {
                r = i;
            }
        }

        // Set directory metadata
        (*dir).pair[0] = pair[(r + 0) % 2];
        (*dir).pair[1] = pair[(r + 1) % 2];
        (*dir).rev = revs[(r + 0) % 2];
        (*dir).off = 0;

        // Scan tags
        for _ in 0..2 {
            let mut off = 0;
            let mut ptag = 0xffffffffu32;

            let mut tempcount = 0u16;
            let mut temptail = [LFS_BLOCK_NULL, LFS_BLOCK_NULL];
            let mut tempsplit = false;
            let mut tempbesttag = besttag;

            let mut maybeerased = false;
            let mut hasfcrc = false;
            let mut fcrc = LfsFcrc {
                size: 0,
                crc: 0,
            };

            // Calculate initial CRC
            let rev_be = lfs_tole32((*dir).rev);
            let mut crc = lfs_crc(0xffffffff, &rev_be as *const _ as *const c_void, 4);
            (*dir).rev = lfs_fromle32((*dir).rev);

            loop {
                // Read tag
                let mut tag = 0u32;
                off += lfs_tag_dsize(ptag) as LfsOff;
                let err = lfs_bd_read(
                    lfs,
                    std::ptr::null_mut(),
                    &mut (*lfs).rcache,
                    (*lfs).cfg.as_ref().unwrap().block_size,
                    (*dir).pair[0],
                    off,
                    &mut tag as *mut _ as *mut c_void,
                    4,
                );

                if err != 0 {
                    if err == LfsError::Corrupt as i32 {
                        break;
                    }
                    return err as LfsStag;
                }

                crc = lfs_crc(crc, &tag as *const _ as *const c_void, 4);
                tag = lfs_frombe32(tag) ^ ptag;

                if !lfs_tag_isvalid(tag) {
                    maybeerased = lfs_tag_type2(ptag) == LfsType::CCrc as u32;
                    break;
                } else if off + lfs_tag_dsize(tag) as LfsOff
                    > (*lfs).cfg.as_ref().unwrap().block_size
                {
                    break;
                }

                ptag = tag;

                match lfs_tag_type2(tag) {
                    x if x == LfsType::CCrc as u32 => {
                        // Handle CRC check
                        let mut dcrc = 0u32;
                        let err = lfs_bd_read(
                            lfs,
                            std::ptr::null_mut(),
                            &mut (*lfs).rcache,
                            (*lfs).cfg.as_ref().unwrap().block_size,
                            (*dir).pair[0],
                            off + 4,
                            &mut dcrc as *mut _ as *mut c_void,
                            4,
                        );
                        dcrc = lfs_fromle32(dcrc);

                        if err != 0 {
                            if err == LfsError::Corrupt as i32 {
                                break;
                            }
                            return err as LfsStag;
                        }

                        if crc != dcrc {
                            break;
                        }

                        // Update directory state
                        besttag = tempbesttag;
                        (*dir).off = off + lfs_tag_dsize(tag) as LfsOff;
                        (*dir).etag = ptag;
                        (*dir).count = tempcount;
                        (*dir).tail = temptail;
                        (*dir).split = tempsplit;

                        // Reset CRC
                        crc = 0xffffffff;
                    }
                    _ => {
                        // Handle other tag types
                        let err = lfs_bd_crc(
                            lfs,
                            std::ptr::null_mut(),
                            &mut (*lfs).rcache,
                            (*lfs).cfg.as_ref().unwrap().block_size,
                            (*dir).pair[0],
                            off + 4,
                            (lfs_tag_dsize(tag) - 4) as LfsSize,
                            &mut crc,
                        );
                        if err != 0 {
                            if err == LfsError::Corrupt as i32 {
                                break;
                            }
                            return err as LfsStag;
                        }

                        // Process different tag types
                        match lfs_tag_type1(tag) {
                            x if x == LfsType::Name as u32 => {
                                if lfs_tag_id(tag) >= tempcount as u32 {
                                    tempcount = lfs_tag_id(tag) as u16 + 1;
                                }
                            }
                            x if x == LfsType::Splice as u32 => {
                                tempcount = (tempcount as i32 + lfs_tag_splice(tag) as i32) as u16;
                            }
                            _ => {}
                        }

                        // Handle FCrc type
                        if lfs_tag_type3(tag) == LfsType::FCrc as u32 {
                            let err = lfs_bd_read(
                                lfs,
                                std::ptr::null_mut(),
                                &mut (*lfs).rcache,
                                (*lfs).cfg.as_ref().unwrap().block_size,
                                (*dir).pair[0],
                                off + 4,
                                &mut fcrc as *mut _ as *mut c_void,
                                std::mem::size_of::<LfsFcrc>() as LfsSize,
                            );
                            lfs_fcrc_fromle32(&mut fcrc);
                            hasfcrc = true;
                        }

                        // Handle callback
                        if (fmask & tag) == (fmask & ftag) {
                            let diskoff = LfsDiskoff {
                                block: (*dir).pair[0],
                                off: off + 4,
                            };
                            let res = cb.unwrap()(
                                data,
                                tag,
                                &diskoff as *const _ as *const c_void,
                            );

                            if res < 0 {
                                if res == LfsError::Corrupt as i32 {
                                    break;
                                }
                                return res as LfsStag;
                            }
                        }
                    }
                }
            }

            // Handle no valid commits
            if (*dir).off == 0 {
                lfs_pair_swap(&mut (*dir).pair);
                (*dir).rev = revs[(r + 1) % 2];
                continue;
            }

            // Final checks and return
            if id != std::ptr::null_mut() {
                *id = lfs_min(lfs_tag_id(besttag as u32), (*dir).count as u32) as u16;
            }

            return if lfs_tag_isvalid(besttag as u32) {
                besttag
            } else if lfs_tag_id(besttag as u32) < (*dir).count as u32 {
                LfsError::NoEnt as LfsStag
            } else {
                0
            };
        }

        LfsError::Corrupt as LfsStag
    }
    implementation
};

//  lfs_dir_fetch
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_fetch(
    lfs: *mut Lfs,
    dir: *mut LfsMdir,
    pair: [LfsBlock; 2],
) -> i32 {
    lfs_dir_fetchmatch(
        lfs,
        dir,
        pair,
        u32::MAX,
        u32::MAX,
        core::ptr::null(),
        core::ptr::null_mut(),
        core::ptr::null_mut(),
    )
}

//  lfs_dir_getgstate
pub(crate) fn lfs_dir_getgstate(
    lfs: &mut Lfs,
    dir: &LfsMdir,
    gstate: &mut LfsGstate,
) -> Result<(), LfsError> {
    let mut temp = LfsGstate {
        tag: 0,
        pair: [0, 0],
    };

    let res = lfs_dir_get(
        lfs,
        dir,
        lfs_util::mktag(0x7ff, 0, 0),
        lfs_util::mktag(LfsType::MoveState as u32, 0, core::mem::size_of::<LfsGstate>() as u32),
        &mut temp,
    );

    if res < 0 && res != LfsError::NoEnt as i32 {
        return Err(unsafe { core::mem::transmute(res) });
    }

    if res != LfsError::NoEnt as i32 {
        // xor together to find resulting gstate
        lfs_gstate_fromle32(&mut temp);
        lfs_gstate_xor(gstate, &temp);
    }

    Ok(())
}

//  lfs_dir_getinfo
static mut lfs_dir_getinfo: unsafe extern "C" fn(
    lfs: *mut Lfs,
    dir: *mut LfsMdir,
    id: u16,
    info: *mut LfsInfo,
) -> i32 = {
    unsafe extern "C" fn implementation(
        lfs: *mut Lfs,
        dir: *mut LfsMdir,
        id: u16,
        info: *mut LfsInfo,
    ) -> i32 {
        if id == 0x3ff {
            // 特殊处理根目录
            let root_name = b"/\0";
            (*info).name[..2].copy_from_slice(root_name);
            (*info).type_ = LfsType::Dir as u8;
            return 0;
        }

        // 获取名称标签
        let tag = lfs_dir_get(
            lfs,
            dir,
            LFS_MKTAG(0x780, 0x3ff, 0),
            LFS_MKTAG(LfsType::Name as u32, id as u32, (*(*lfs).name_max + 1) as u32),
            (*info).name.as_mut_ptr() as *mut c_void,
        );
        if tag < 0 {
            return tag as i32;
        }

        // 设置文件类型
        (*info).type_ = lfs_tag_type3(tag) as u8;

        // 获取结构信息
        let mut ctz = LfsCtz {
            head: 0,
            size: 0,
        };
        let tag = lfs_dir_get(
            lfs,
            dir,
            LFS_MKTAG(0x700, 0x3ff, 0),
            LFS_MKTAG(LfsType::Struct as u32, id as u32, std::mem::size_of::<LfsCtz>() as u32),
            &mut ctz as *mut LfsCtz as *mut c_void,
        );
        if tag < 0 {
            return tag as i32;
        }
        lfs_ctz_fromle32(&mut ctz);

        // 根据结构类型设置大小
        match lfs_tag_type3(tag) as u32 {
            x if x == LfsType::CtzStruct as u32 => {
                (*info).size = ctz.size;
            }
            x if x == LfsType::InlineStruct as u32 => {
                (*info).size = lfs_tag_size(tag) as LfsSize;
            }
            _ => {}
        }

        0
    }

    implementation
};

//  lfs_dir_find_match
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_find_match(
    data: *mut c_void,
    tag: LfsTag,
    buffer: *const c_void,
) -> i32 {
    let name = &mut *(data as *mut LfsDirFindMatch);
    let lfs = name.lfs;
    let disk = &*(buffer as *const LfsDiskoff);

    let diff = core::cmp::min(name.size, lfs_tag_size(tag));
    let res = lfs_bd_cmp(
        lfs,
        core::ptr::null_mut(),
        &mut (*lfs).rcache,
        diff,
        disk.block,
        disk.off,
        name.name as *const c_void,
        diff,
    );

    if res != LfsCmp::Eq as i32 {
        return res;
    }

    if name.size != lfs_tag_size(tag) {
        return if name.size < lfs_tag_size(tag) {
            LfsCmp::Lt as i32
        } else {
            LfsCmp::Gt as i32
        };
    }

    LfsCmp::Eq as i32
}

//  lfs_dir_find
pub(crate) fn lfs_dir_find(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    path: &mut &[u8],
    id: &mut u16,
) -> LfsStag {
    // 初始化为根目录
    let mut name = *path;
    let mut tag = LFS_MKTAG!(LfsType::Dir, 0x3ff, 0);
    dir.tail[0] = lfs.root[0];
    dir.tail[1] = lfs.root[1];

    // 空路径检查
    if name.is_empty() {
        return LfsError::Inval as LfsStag;
    }

    loop {
        // 跳过目录分隔符
        if lfs_tag_type3(tag) == LfsType::Dir as u32 {
            name = &name[name.iter().take_while(|&&c| c == b'/').count()..];
        }

        // 提取当前路径段
        let namelen = name.iter().take_while(|&&c| c != b'/').count();
        let current = name.get(..namelen).unwrap_or(&[]);

        // 处理特殊目录项
        match current {
            b"." if namelen == 1 => {
                name = &name[namelen..];
                continue;
            }
            b".." if namelen == 2 => {
                return LfsError::Inval as LfsStag;
            }
            _ => {}
        }

        // 处理路径中的'..'跳转
        let mut suffix = &name[namelen..];
        let mut depth = 1;
        loop {
            suffix = &suffix[suffix.iter().take_while(|&&c| c == b'/').count()..];
            let seglen = suffix.iter().take_while(|&&c| c != b'/').count();
            if seglen == 0 {
                break;
            }

            match suffix.get(..seglen) {
                Some(b".") => (),
                Some(b"..") => {
                    depth -= 1;
                    if depth == 0 {
                        name = &suffix[seglen..];
                        continue 'nextname;
                    }
                }
                _ => depth += 1,
            }

            suffix = &suffix[seglen..];
        }

        // 找到最终路径项
        if name.is_empty() {
            return tag;
        }

        // 更新路径指针
        *path = name;

        // 必须为目录类型
        if lfs_tag_type3(tag) != LfsType::Dir as u32 {
            return LfsError::NotDir as LfsStag;
        }

        // 获取目录元数据
        if lfs_tag_id(tag) != 0x3ff {
            let res = lfs_dir_get(
                lfs,
                dir,
                LFS_MKTAG!(0x700, 0x3ff, 0),
                LFS_MKTAG!(LfsType::Struct, lfs_tag_id(tag), 8),
                &mut dir.tail,
            );
            if res < 0 {
                return res;
            }
            lfs_pair_fromle32(&mut dir.tail);
        }

        // 查找匹配项
        loop {
            let mut find = LfsDirFindMatch {
                lfs,
                name: current.as_ptr() as *const c_void,
                size: namelen as LfsSize,
            };
            
            tag = lfs_dir_fetchmatch(
                lfs,
                dir,
                &dir.tail,
                LFS_MKTAG!(0x780, 0, 0),
                LFS_MKTAG!(LfsType::Name, 0, namelen as u32),
                id,
                Some(lfs_dir_find_match),
                &mut find as *mut _ as *mut c_void,
            );

            if tag < 0 {
                return tag;
            }

            if tag != 0 {
                break;
            }

            if !dir.split {
                return LfsError::NoEnt as LfsStag;
            }
        }

        // 移动到下一路径段
        name = &name[namelen..];
    }
}

// 跳转标签模拟
'nextname: loop {
    break 'nextname;
}

//  lfs_dir_commitprog
pub(crate) fn lfs_dir_commitprog(
    lfs: &mut Lfs,
    commit: &mut LfsCommit,
    buffer: *const c_void,
    size: LfsSize,
) -> i32 {
    let err = lfs_bd_prog(
        lfs,
        &mut lfs.pcache,
        &mut lfs.rcache,
        false,
        commit.block,
        commit.off,
        buffer as *const u8,
        size,
    );
    if err != 0 {
        return err;
    }

    commit.crc = lfs_crc(commit.crc, buffer as *const u8, size);
    commit.off += size;
    0
}

//  lfs_dir_commitattr
pub fn lfs_dir_commitattr(
    lfs: &mut Lfs,
    commit: &mut LfsCommit,
    tag: LfsTag,
    buffer: *const c_void,
) -> Result<(), LfsError> {
    let dsize = lfs_util::lfs_tag_dsize(tag);
    if commit.off + dsize > commit.end {
        return Err(LfsError::NoSpc);
    }

    // Convert tag to big-endian and XOR with previous tag
    let ntag = ((tag & 0x7fffffff) ^ commit.ptag).to_be();
    lfs_dir_commitprog(lfs, commit, &ntag as *const _ as *const c_void, 4)?;

    // Handle different buffer types based on tag flag
    if (tag & 0x80000000) == 0 {
        // Write directly from memory buffer
        let data_size = (dsize - 4) as usize;
        lfs_dir_commitprog(lfs, commit, buffer, data_size)?;
    } else {
        // Read from disk block and write incrementally
        let disk = unsafe { &*(buffer as *const LfsDiskoff) };
        let data_size = dsize - 4;
        
        for i in 0..data_size {
            let mut dat = 0u8;
            lfs_bd_read(
                lfs,
                None,
                &mut lfs.rcache,
                data_size - i,
                disk.block,
                disk.off + i,
                &mut dat as *mut _ as *mut c_void,
                1,
            )?;
            
            lfs_dir_commitprog(lfs, commit, &dat as *const _ as *const c_void, 1)?;
        }
    }

    commit.ptag = tag & 0x7fffffff;
    Ok(())
}

//  lfs_dir_commitcrc
pub(crate) fn lfs_dir_commitcrc(lfs: &mut Lfs, commit: &mut LfsCommit) -> Result<(), LfsError> {
    // align to program units
    let end = lfs_util::align_up(
        (commit.off + 5 * core::mem::size_of::<u32>() as LfsOff).min(lfs.cfg.block_size),
        lfs.cfg.prog_size,
    );

    let mut off1 = 0;
    let mut crc1 = 0;

    while commit.off < end {
        let tag_size = core::mem::size_of::<LfsTag>() as LfsOff;
        let mut noff = (end - (commit.off + tag_size)).min(0x3fe) + (commit.off + tag_size);
        if noff < end {
            noff = noff.min(end - 5 * core::mem::size_of::<u32>() as LfsOff);
        }

        let mut eperturb: u8 = 0xff;
        if noff >= end && noff <= lfs.cfg.block_size - lfs.cfg.prog_size {
            let err = lfs_bd_read(
                lfs,
                None,
                &mut lfs.rcache,
                lfs.cfg.prog_size,
                commit.block,
                noff,
                &mut eperturb as *mut _ as *mut c_void,
                1,
            );
            match err {
                Ok(()) | Err(LfsError::Corrupt) => {}
                Err(e) => return Err(e),
            }

            #[cfg(feature = "multiversion")]
            if lfs_fs_disk_version(lfs) <= 0x00020000 {
                // skip writing fcrc
            } else {
                let mut fcrc = LfsFcrc {
                    size: lfs.cfg.prog_size,
                    crc: 0xffffffff,
                };
                let err = lfs_bd_crc(
                    lfs,
                    None,
                    &mut lfs.rcache,
                    lfs.cfg.prog_size,
                    commit.block,
                    noff,
                    fcrc.size,
                    &mut fcrc.crc,
                );
                match err {
                    Ok(()) | Err(LfsError::Corrupt) => {}
                    Err(e) => return Err(e),
                }

                lfs_fcrc_tole32(&mut fcrc);
                lfs_dir_commitattr(
                    lfs,
                    commit,
                    LFS_MKTAG(LfsType::FCrc, 0x3ff, core::mem::size_of::<LfsFcrc>() as u32),
                    &fcrc,
                )?;
            }
        }

        let ntag = LFS_MKTAG(
            (LfsType::CCrc as u32).wrapping_add((((!eperturb) >> 7) as u32)),
            0x3ff,
            noff - (commit.off + tag_size),
        );

        let mut ccrc_tag = (ntag ^ commit.ptag).to_be();
        commit.crc = lfs_crc(commit.crc, &ccrc_tag.to_ne_bytes(), core::mem::size_of::<LfsTag>());
        let ccrc_crc = commit.crc.to_le();

        let ccrc = [ccrc_tag.to_be_bytes(), ccrc_crc.to_le_bytes()].concat();
        lfs_bd_prog(
            lfs,
            &mut lfs.pcache,
            &mut lfs.rcache,
            false,
            commit.block,
            commit.off,
            ccrc.as_ptr() as *const c_void,
            ccrc.len() as LfsSize,
        )?;

        if off1 == 0 {
            off1 = commit.off + tag_size;
            crc1 = commit.crc;
        }

        commit.off = noff;
        commit.ptag = ntag ^ ((0x80 & !eperturb) as u32).wrapping_shl(24);
        commit.crc = 0xffffffff;

        if noff >= end || noff >= lfs.pcache.off + lfs.cfg.cache_size {
            lfs_bd_sync(lfs, &mut lfs.pcache, &mut lfs.rcache, false)?;
        }
    }

    let mut crc = 0xffffffff;
    lfs_bd_crc(
        lfs,
        None,
        &mut lfs.rcache,
        off1 + core::mem::size_of::<u32>() as LfsOff,
        commit.block,
        commit.begin,
        off1 - commit.begin,
        &mut crc,
    )?;

    if crc != crc1 {
        return Err(LfsError::Corrupt);
    }

    lfs_bd_crc(
        lfs,
        None,
        &mut lfs.rcache,
        core::mem::size_of::<u32>() as LfsOff,
        commit.block,
        off1,
        core::mem::size_of::<u32>() as LfsOff,
        &mut crc,
    )?;

    if crc != 0 {
        return Err(LfsError::Corrupt);
    }

    Ok(())
}

//  lfs_dir_alloc
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_alloc(lfs: *mut Lfs, dir: *mut LfsMdir) -> LfsError {
    // allocate pair of dir blocks (backwards, so we write block 1 first)
    for i in 0..2 {
        let err = lfs_alloc(lfs, &mut (*dir).pair[(i + 1) % 2]);
        if err != LfsError::Ok {
            return err;
        }
    }

    // zero for reproducibility in case initial block is unreadable
    (*dir).rev = 0;

    // rather than clobbering one of the blocks we just pretend
    // the revision may be valid
    let mut rev = 0u32;
    let err = lfs_bd_read(
        lfs,
        core::ptr::null_mut(),
        &mut (*lfs).rcache,
        core::mem::size_of_val(&rev) as LfsSize,
        (*dir).pair[0],
        0,
        &mut rev as *mut _ as *mut c_void,
        core::mem::size_of_val(&rev) as LfsSize,
    );
    (*dir).rev = lfs_fromle32(rev);
    if err != LfsError::Ok && err != LfsError::Corrupt {
        return err;
    }

    // to make sure we don't immediately evict, align the new revision count
    // to our block_cycles modulus, see lfs_dir_compact for why our modulus
    // is tweaked this way
    if (*lfs).cfg.as_ref().unwrap().block_cycles > 0 {
        let cycles = ((*lfs).cfg.as_ref().unwrap().block_cycles + 1) | 1;
        (*dir).rev = lfs_alignup((*dir).rev, cycles as u32);
    }

    // set defaults
    (*dir).off = core::mem::size_of_val(&(*dir).rev) as LfsOff;
    (*dir).etag = 0xffffffff;
    (*dir).count = 0;
    (*dir).tail[0] = LFS_BLOCK_NULL;
    (*dir).tail[1] = LFS_BLOCK_NULL;
    (*dir).erased = false;
    (*dir).split = false;

    // don't write out yet, let caller take care of that
    LfsError::Ok
}

//  lfs_dir_drop
pub unsafe fn lfs_dir_drop(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    tail: &mut LfsMdir,
) -> Result<(), LfsError> {
    // steal state
    lfs_dir_getgstate(lfs, tail, &mut lfs.gdelta)?;

    // steal tail
    lfs_pair_tole32(&mut tail.tail);
    let type_ = LfsType::Tail as u16 + tail.split as u16;
    let tag = lfs_mktag(type_, 0x3ff, 8);
    let attrs = [LfsMattr {
        tag,
        buffer: tail.tail.as_ptr() as *const c_void,
    }];
    lfs_dir_commit(lfs, dir, &attrs)?;
    lfs_pair_fromle32(&mut tail.tail);

    Ok(())
}

//  lfs_dir_split
pub fn lfs_dir_split(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    attrs: *const LfsMattr,
    attrcount: i32,
    source: &mut LfsMdir,
    split: u16,
    end: u16,
) -> Result<(), LfsError> {
    // 创建尾元数据对
    let mut tail = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };

    lfs_dir_alloc(lfs, &mut tail)?;

    // 继承分裂状态和尾部指针
    tail.split = dir.split;
    tail.tail.copy_from_slice(&dir.tail);

    // 执行目录压缩操作（忽略LFS_OK_RELOCATED状态）
    let res = lfs_dir_compact(lfs, &mut tail, attrs, attrcount, source, split, end);
    if res < 0 {
        return Err(LfsError::from(res));
    }

    // 更新目录元数据
    dir.tail.copy_from_slice(&tail.pair);
    dir.split = true;

    // 检查是否需要更新根目录
    if lfs_pair_cmp(&dir.pair, &lfs.root) == LfsCmp::Eq && split == 0 {
        lfs.root.copy_from_slice(&tail.pair);
    }

    Ok(())
}

//  lfs_dir_commit_size
/// 计算目录提交所需空间的回调函数
/// 参数：
/// - p: 用户提供的指针，此处为指向大小变量的指针
/// - tag: 目录条目标签
/// - buffer: 未使用的数据缓冲区
/// 返回操作结果(LfsError枚举)
#[no_mangle]
pub extern "C" fn lfs_dir_commit_size(p: *mut c_void, tag: LfsTag, buffer: *const c_void) -> LfsError {
    let size = unsafe { &mut *(p as *mut LfsSize) };
    let _ = buffer; // 显式忽略未使用参数

    *size += lfs_tag_dsize(tag);
    LfsError::Ok
}

//  lfs_dir_commit_commit
pub unsafe extern "C" fn lfs_dir_commit_commit(
    p: *mut c_void,
    tag: LfsTag,
    buffer: *const c_void,
) -> i32 {
    let commit = p as *mut LfsDirCommitCommit;
    lfs_dir_commitattr((*commit).lfs, (*commit).commit, tag, buffer)
}

//  lfs_dir_needsrelocation
pub(crate) fn lfs_dir_needsrelocation(lfs: &Lfs, dir: &LfsMdir) -> bool {
    unsafe {
        // 使用安全断言确保配置存在
        let cfg = &*lfs.cfg;
        cfg.block_cycles > 0 && {
            let mod_value = (cfg.block_cycles as u32 + 1) | 1;
            (dir.rev + 1) % mod_value == 0
        }
    }
}

//  lfs_dir_compact
pub(crate) fn lfs_dir_compact(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    attrs: *const LfsMattr,
    attrcount: i32,
    source: &LfsMdir,
    begin: u16,
    end: u16,
) -> Result<i32, LfsError> {
    let mut relocated = false;
    let mut tired = lfs_dir_needsrelocation(lfs, dir);

    dir.rev += 1;

    #[cfg(feature = "migrate")]
    if !lfs.lfs1.is_null() {
        tired = false;
    }

    if tired && lfs_pair_cmp(&dir.pair, &[0, 1]) != LfsCmp::Eq {
        return Err(LfsError::Corrupt); // 触发重定位
    }

    'main: loop {
        let mut commit = LfsCommit {
            block: dir.pair[1],
            off: 0,
            ptag: 0xffffffff,
            crc: 0xffffffff,
            begin: 0,
            end: if lfs.cfg.metadata_max != 0 {
                lfs.cfg.metadata_max
            } else {
                lfs.cfg.block_size
            } - 8,
        };

        // 擦除块
        if let Err(e) = lfs_bd_erase(lfs, dir.pair[1]) {
            if e == LfsError::Corrupt {
                relocated = true;
                break 'main;
            }
            return Err(e);
        }

        // 写入头部
        let rev_le = dir.rev.to_le();
        if let Err(e) = lfs_dir_commitprog(lfs, &mut commit, &rev_le as *const _ as *const u8, 4) {
            dir.rev = u32::from_le(rev_le);
            if e == LfsError::Corrupt {
                relocated = true;
                break 'main;
            }
            return Err(e);
        }
        dir.rev = u32::from_le(rev_le);

        // 遍历目录
        let mut commit_data = LfsDirCommitCommit { lfs, commit: &mut commit };
        if let Err(e) = lfs_dir_traverse(
            lfs,
            source,
            0,
            0xffffffff,
            attrs,
            attrcount,
            lfs_mktag(0x400, 0x3ff, 0),
            lfs_mktag(LfsType::Name as u32, 0, 0),
            begin,
            end,
            -(begin as i16),
            Some(lfs_dir_commit_commit),
            &mut commit_data as *mut _ as *mut c_void,
        ) {
            if e == LfsError::Corrupt {
                relocated = true;
                break 'main;
            }
            return Err(e);
        }

        // 处理tail
        if !lfs_pair_isnull(&dir.tail) {
            let mut tail_le = [0u32; 2];
            tail_le[0] = dir.tail[0].to_le();
            tail_le[1] = dir.tail[1].to_le();
            if let Err(e) = lfs_dir_commitattr(
                lfs,
                &mut commit,
                lfs_mktag(LfsType::Tail as u32 + dir.split as u32, 0x3ff, 8),
                tail_le.as_ptr() as *const c_void,
            ) {
                if e == LfsError::Corrupt {
                    relocated = true;
                    break 'main;
                }
                return Err(e);
            }
            dir.tail[0] = u32::from_le(tail_le[0]);
            dir.tail[1] = u32::from_le(tail_le[1]);
        }

        // 处理gstate
        let mut delta = LfsGstate::default();
        if !relocated {
            lfs_gstate_xor(&mut delta, &lfs.gdisk);
            lfs_gstate_xor(&mut delta, &lfs.gstate);
        }
        lfs_gstate_xor(&mut delta, &lfs.gdelta);
        delta.tag &= !lfs_mktag(0, 0, 0x3ff);

        if let Err(e) = lfs_dir_getgstate(lfs, dir, &mut delta) {
            return Err(e);
        }

        if !lfs_gstate_iszero(&delta) {
            let delta_le = delta.tag.to_le();
            if let Err(e) = lfs_dir_commitattr(
                lfs,
                &mut commit,
                lfs_mktag(LfsType::MoveState as u32, 0x3ff, 4),
                &delta_le as *const _ as *const c_void,
            ) {
                if e == LfsError::Corrupt {
                    relocated = true;
                    break 'main;
                }
                return Err(e);
            }
        }

        // 完成提交
        if let Err(e) = lfs_dir_commitcrc(lfs, &mut commit) {
            if e == LfsError::Corrupt {
                relocated = true;
                break 'main;
            }
            return Err(e);
        }

        // 更新目录状态
        dir.pair.swap(0, 1);
        dir.count = end - begin;
        dir.off = commit.off;
        dir.etag = commit.ptag;
        lfs.gdelta = LfsGstate::default();
        if !relocated {
            lfs.gdisk = lfs.gstate.clone();
        }
        break;
    }

    if relocated {
        lfs_cache_drop(lfs, &mut lfs.pcache);
        if lfs_pair_cmp(&dir.pair, &[0, 1]) == LfsCmp::Eq {
            return Err(LfsError::NoSpc);
        }

        if let Err(e) = lfs_alloc(lfs, &mut dir.pair[1]) {
            if e != LfsError::NoSpc || !tired {
                return Err(e);
            }
        }
        return Ok(LfsOk::Relocated as i32);
    }

    Ok(0)
}

//  lfs_dir_splittingcompact
pub unsafe extern "C" fn lfs_dir_splittingcompact(
    lfs: *mut Lfs,
    dir: *mut LfsMdir,
    attrs: *const LfsMattr,
    attrcount: i32,
    source: *mut LfsMdir,
    mut begin: u16,
    mut end: u16,
) -> i32 {
    loop {
        let mut split = begin;
        while end.wrapping_sub(split) > 1 {
            let mut size = 0;
            let err = lfs_dir_traverse(
                lfs,
                source,
                0,
                0xffffffff,
                attrs,
                attaccount,
                LFS_MKTAG(LfsType::Splice as u32, 0x3ff, 0),
                LFS_MKTAG(LfsType::Name as u32, 0, 0),
                split,
                end,
                -(split as i16),
                Some(lfs_dir_commit_size),
                &mut size as *mut _ as *mut c_void,
            );
            if err != 0 {
                return err;
            }

            let metadata_max = if (*lfs).cfg.as_ref().unwrap().metadata_max != 0 {
                (*lfs).cfg.as_ref().unwrap().metadata_max
            } else {
                (*lfs).cfg.as_ref().unwrap().block_size
            };

            let aligned = lfs_util::lfs_alignup(
                metadata_max / 2,
                (*lfs).cfg.as_ref().unwrap().prog_size,
            );
            let limit = lfs_util::lfs_min(metadata_max.wrapping_sub(40), aligned);
            
            if (end - split) < 0xff && size <= limit {
                break;
            }
            
            split += (end - split) / 2;
        }

        if split == begin {
            break;
        }

        let err = lfs_dir_split(lfs, dir, attrs, attaccount, source, split, end);
        if err != 0 && err != LfsError::NoSpc as i32 {
            return err;
        }

        if err == LfsError::NoSpc as i32 {
            // LFS_WARN!("Unable to split {0x%"PRIx32", 0x%"PRIx32"}", dir.pair[0], dir.pair[1]);
            break;
        } else {
            end = split;
        }
    }

    if lfs_dir_needsrelocation(lfs, dir) 
        && lfs_pair_cmp((*dir).pair.as_ptr(), [0, 1].as_ptr()) == LfsCmp::Eq as i32
    {
        let size = lfs_fs_size_(lfs);
        if size < 0 {
            return size as i32;
        }

        if ((*lfs).block_count - size as LfsSize) > (*lfs).block_count / 8 {
            // LFS_DEBUG!("Expanding superblock at rev %"PRIu32, dir.rev);
            let err = lfs_dir_split(lfs, dir, attrs, attaccount, source, begin, end);
            if err != 0 && err != LfsError::NoSpc as i32 {
                return err;
            }

            if err == 0 {
                end = 1;
            }
        }
    }

    lfs_dir_compact(lfs, dir, attrs, attaccount, source, begin, end)
}

//  lfs_dir_relocatingcommit
pub unsafe extern "C" fn lfs_dir_relocatingcommit(
    lfs: *mut Lfs,
    dir: *mut LfsMdir,
    pair: [LfsBlock; 2],
    attrs: *const LfsMattr,
    attrcount: i32,
    pdir: *mut LfsMdir,
) -> i32 {
    let mut state = 0i32;
    let mut hasdelete = false;

    // Calculate changes to the directory
    for i in 0..attrcount {
        let attr = &*attrs.add(i as usize);
        let tag_type = attr.tag & 0x3ff; // lfs_tag_type3

        match tag_type {
            x if x == LfsType::Create as u32 => {
                (*dir).count += 1;
            }
            x if x == LfsType::Delete as u32 => {
                debug_assert!((*dir).count > 0);
                (*dir).count -= 1;
                hasdelete = true;
            }
            x if x == LfsType::Tail as u32 => {
                let buffer = attr.buffer as *const LfsBlock;
                (*dir).tail[0] = *buffer.add(0);
                (*dir).tail[1] = *buffer.add(1);
                (*dir).split = (attr.tag >> 16 & 1) != 0; // lfs_tag_chunk
                lfs_util::lfs_pair_fromle32((*dir).tail.as_mut_ptr());
            }
            _ => {}
        }
    }

    // Check if we should drop the directory block
    if hasdelete && (*dir).count == 0 {
        debug_assert!(!pdir.is_null());
        let err = lfs_fs_pred(lfs, (*dir).pair.as_ptr(), pdir);
        if err != 0 && err != LfsError::NoEnt as i32 {
            return err;
        }

        if err != LfsError::NoEnt as i32 && (*pdir).split {
            state = LfsOk::Dropped as i32;
            unsafe { goto fixmlist; }
        }
    }

    if (*dir).erased {
        // Prepare commit structure
        let metadata_max = if (*(*lfs).cfg).metadata_max != 0 {
            (*(*lfs).cfg).metadata_max
        } else {
            (*(*lfs).cfg).block_size
        };
        let mut commit = LfsCommit {
            block: (*dir).pair[0],
            off: (*dir).off,
            ptag: (*dir).etag,
            crc: 0xffffffff,
            begin: (*dir).off,
            end: metadata_max - 8,
        };

        // Traverse attributes
        lfs_util::lfs_pair_tole32((*dir).tail.as_mut_ptr());
        let commit_data = LfsDirCommitCommit { lfs, commit: &mut commit };
        let err = lfs_dir_traverse(
            lfs,
            dir,
            (*dir).off,
            (*dir).etag,
            attrs,
            attrcount,
            0, 0, 0, 0, 0,
            Some(lfs_dir_commit_commit),
            &commit_data as *const _ as *mut c_void,
        );
        lfs_util::lfs_pair_fromle32((*dir).tail.as_mut_ptr());

        if err != 0 {
            if err == LfsError::NoSpc as i32 || err == LfsError::Corrupt as i32 {
                unsafe { goto compact; }
            }
            return err;
        }

        // Handle global state
        let mut delta = LfsGstate::default();
        lfs_gstate_xor(&mut delta, &(*lfs).gstate);
        lfs_gstate_xor(&mut delta, &(*lfs).gdisk);
        lfs_gstate_xor(&mut delta, &(*lfs).gdelta);
        delta.tag &= !0x3ff;

        if !lfs_gstate_iszero(&delta) {
            let err = lfs_dir_getgstate(lfs, dir, &mut delta);
            if err != 0 {
                return err;
            }

            lfs_util::lfs_gstate_tole32(&mut delta);
            let err = lfs_dir_commitattr(
                lfs,
                &mut commit,
                (LfsType::MoveState as u32) | (0x3ff << 16) | ((core::mem::size_of::<LfsGstate>() as u32) << 8),
                &delta as *const _ as *const c_void,
            );
            if err != 0 {
                if err == LfsError::NoSpc as i32 || err == LfsError::Corrupt as i32 {
                    unsafe { goto compact; }
                }
                return err;
            }
        }

        // Finalize commit
        let err = lfs_dir_commitcrc(lfs, &mut commit);
        if err != 0 {
            if err == LfsError::NoSpc as i32 || err == LfsError::Corrupt as i32 {
                unsafe { goto compact; }
            }
            return err;
        }

        // Update directory state
        (*dir).off = commit.off;
        (*dir).etag = commit.ptag;
        (*lfs).gdisk = (*lfs).gstate.clone();
        (*lfs).gdelta = LfsGstate::default();

        state = LfsOk::Dropped as i32;
        unsafe { goto fixmlist; }
    }

    // Compact phase
    compact:
    lfs_cache_drop(lfs, &mut (*lfs).pcache);
    state = lfs_dir_splittingcompact(
        lfs,
        dir,
        attrs,
        attrcount,
        dir,
        0,
        (*dir).count as i32,
    );
    if state < 0 {
        return state;
    }

    // Fix metadata list
    fixmlist:
    let oldpair = [pair[0], pair[1]];
    let mut d = (*lfs).mlist;
    while !d.is_null() {
        let curr = &mut *d;
        if lfs_pair_cmp(curr.m.pair, oldpair) == LfsCmp::Eq as i32 {
            curr.m = *dir;

            if curr.m.pair.as_ptr() != pair.as_ptr() {
                for i in 0..attrcount {
                    let attr = &*attrs.add(i as usize);
                    let tag_type = attr.tag & 0x3ff;

                    if tag_type == LfsType::Delete as u32 {
                        let id = (attr.tag >> 16) & 0x3ff;
                        if curr.id == id as u16 {
                            curr.m.pair = [LFS_BLOCK_NULL; 2];
                        } else if curr.id > id as u16 {
                            curr.id -= 1;
                            if curr.type_ == LfsType::Dir as u8 {
                                (*(d as *mut LfsDir)).pos -= 1;
                            }
                        }
                    } else if tag_type == LfsType::Create as u32 {
                        let id = (attr.tag >> 16) & 0x3ff;
                        if curr.id >= id as u16 {
                            curr.id += 1;
                            if curr.type_ == LfsType::Dir as u8 {
                                (*(d as *mut LfsDir)).pos += 1;
                            }
                        }
                    }
                }
            }

            while curr.id >= curr.m.count as u16 && curr.m.split {
                if lfs_pair_cmp(curr.m.tail, (*lfs).root) != LfsCmp::Eq as i32 {
                    curr.id -= curr.m.count as u16;
                }
                let err = lfs_dir_fetch(lfs, &mut curr.m, curr.m.tail.as_ptr());
                if err != 0 {
                    return err;
                }
            }
        }
        d = curr.next;
    }

    state
}

//  lfs_dir_orphaningcommit
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_orphaningcommit(
    lfs: *mut Lfs,
    dir: *mut LfsMdir,
    attrs: *const LfsMattr,
    attrcount: i32,
) -> i32 {
    let lfs = &mut *lfs;
    let dir_ref = &mut *dir;

    // 处理内联文件
    let mut f = lfs.mlist as *mut LfsMlist;
    while !f.is_null() {
        let file = &mut *(f as *mut LfsFile);
        if dir_ref as *const _ != &file.m as *const _
            && lfs_util::lfs_pair_cmp((*file).m.pair, dir_ref.pair) == LfsCmp::Eq as i32
            && file.type_ == LfsType::Reg as u8
            && (file.flags & LfsOpenFlags::Inline as u32) != 0
            && file.ctz.size > (*lfs.cfg).cache_size
        {
            let err = lfs_file_outline(lfs, file as *mut LfsFile);
            if err != 0 {
                return err;
            }

            let err = lfs_file_flush(lfs, file as *mut LfsFile);
            if err != 0 {
                return err;
            }
        }
        f = (*f).next;
    }

    // 保存原始pair并创建本地副本
    let lpair = [(*dir).pair[0], (*dir).pair[1]];
    let mut ldir = dir_ref.clone();
    let mut pdir = LfsMdir::default();

    // 执行初始提交操作
    let mut state = lfs_dir_relocatingcommit(
        lfs,
        &mut ldir,
        dir_ref.pair.as_mut_ptr(),
        attrs,
        attrcount,
        &mut pdir,
    );
    if state < 0 {
        return state;
    }

    // 更新目录引用
    if lfs_util::lfs_pair_cmp(dir_ref.pair, lpair.as_ptr()) == LfsCmp::Eq as i32 {
        *dir_ref = ldir.clone();
    }

    // 处理删除状态
    if state == LfsOk::Dropped as i32 {
        let err = lfs_dir_getgstate(lfs, dir_ref, &mut lfs.gdelta);
        if err != 0 {
            return err;
        }

        // 处理尾块提交
        let mut new_pair = [pdir.pair[0], pdir.pair[1]];
        lfs_util::lfs_pair_tole32(dir_ref.tail.as_mut_ptr());
        state = lfs_dir_relocatingcommit(
            lfs,
            &mut pdir,
            new_pair.as_mut_ptr(),
            LfsMattr::as_ptr(&LfsMattr {
                tag: lfs_util::LFS_MKTAG(
                    LfsType::Tail as u32 + dir_ref.split as u32,
                    0x3ff,
                    8,
                ),
                buffer: dir_ref.tail.as_ptr() as *const c_void,
            }),
            1,
        );
        lfs_util::lfs_pair_fromle32(dir_ref.tail.as_mut_ptr());
        if state < 0 {
            return state;
        }

        ldir = pdir.clone();
    }

    // 处理重定位循环
    let mut orphans = false;
    while state == LfsOk::Relocated as i32 {
        orphans = true;
        lfs_util::lfs_debug(
            b"Relocating {0x%"PRIx32", 0x%"PRIx32"} -> {0x%"PRIx32", 0x%"PRIx32"}" as *const u8,
            lpair[0],
            lpair[1],
            ldir.pair[0],
            ldir.pair[1],
        );

        // 更新根目录引用
        if lfs_util::lfs_pair_cmp(lpair.as_ptr(), lfs.root.as_ptr()) == LfsCmp::Eq as i32 {
            lfs.root[0] = ldir.pair[0];
            lfs.root[1] = ldir.pair[1];
        }

        // 更新所有mlist中的目录引用
        let mut d = lfs.mlist;
        while !d.is_null() {
            let mlist = &mut *d;
            if lfs_util::lfs_pair_cmp(lpair.as_ptr(), mlist.m.pair.as_ptr()) == LfsCmp::Eq as i32 {
                mlist.m.pair[0] = ldir.pair[0];
                mlist.m.pair[1] = ldir.pair[1];
            }

            if mlist.type_ == LfsType::Dir as u8
                && lfs_util::lfs_pair_cmp(lpair.as_ptr(), (*(d as *mut LfsDir)).head.as_ptr())
                    == LfsCmp::Eq as i32
            {
                (*(d as *mut LfsDir)).head[0] = ldir.pair[0];
                (*(d as *mut LfsDir)).head[1] = ldir.pair[1];
            }
            d = (*d).next;
        }

        // 查找父目录
        let mut parent_pair = [0; 2];
        let tag = lfs_fs_parent(lfs, lpair.as_ptr(), &mut pdir);
        let hasparent = tag != LfsError::NoEnt as i32;
        if tag < 0 && tag != LfsError::NoEnt as i32 {
            return tag;
        }

        if hasparent {
            // 准备孤儿计数
            let err = lfs_fs_preporphans(lfs, 1);
            if err != 0 {
                return err;
            }

            // 处理移动标记
            let mut moveid = 0x3ff;
            if lfs_gstate_hasmovehere(&lfs.gstate, pdir.pair.as_ptr()) {
                moveid = lfs_util::lfs_tag_id(lfs.gstate.tag);
                lfs_fs_prepmove(lfs, 0x3ff, core::ptr::null_mut());
                // TODO: 处理tag调整逻辑
            }

            // 提交父目录更新
            let mut ppair = [pdir.pair[0], pdir.pair[1]];
            lfs_util::lfs_pair_tole32(ldir.pair.as_mut_ptr());
            state = lfs_dir_relocatingcommit(
                lfs,
                &mut pdir,
                ppair.as_mut_ptr(),
                LfsMattr::as_ptr(&[
                    LfsMattr {
                        tag: if moveid != 0x3ff {
                            lfs_util::LFS_MKTAG(LfsType::Delete as u32, moveid, 0)
                        } else {
                            0
                        },
                        buffer: core::ptr::null(),
                    },
                    LfsMattr {
                        tag: tag as u32,
                        buffer: ldir.pair.as_ptr() as *const c_void,
                    },
                ]),
                2,
            );
            lfs_util::lfs_pair_fromle32(ldir.pair.as_mut_ptr());
            if state < 0 {
                return state;
            }

            if state == LfsOk::Relocated as i32 {
                lpair[0] = ppair[0];
                lpair[1] = ppair[1];
                ldir = pdir.clone();
                continue;
            }
        }

        // 查找前驱目录
        let err = lfs_fs_pred(lfs, lpair.as_ptr(), &mut pdir);
        if err != 0 && err != LfsError::NoEnt as i32 {
            return err;
        }

        if err == 0 {
            // 清理孤儿计数
            if lfs_gstate_hasorphans(&lfs.gstate) {
                let err = lfs_fs_preporphans(lfs, -hasparent as i32);
                if err != 0 {
                    return err;
                }
            }

            // 处理移动标记
            let mut moveid = 0x3ff;
            if lfs_gstate_hasmovehere(&lfs.gstate, pdir.pair.as_ptr()) {
                moveid = lfs_util::lfs_tag_id(lfs.gstate.tag);
                lfs_fs_prepmove(lfs, 0x3ff, core::ptr::null_mut());
            }

            // 提交前驱目录更新
            lpair[0] = pdir.pair[0];
            lpair[1] = pdir.pair[1];
            lfs_util::lfs_pair_tole32(ldir.pair.as_mut_ptr());
            state = lfs_dir_relocatingcommit(
                lfs,
                &mut pdir,
                lpair.as_mut_ptr(),
                LfsMattr::as_ptr(&[
                    LfsMattr {
                        tag: if moveid != 0x3ff {
                            lfs_util::LFS_MKTAG(LfsType::Delete as u32, moveid, 0)
                        } else {
                            0
                        },
                        buffer: core::ptr::null(),
                    },
                    LfsMattr {
                        tag: lfs_util::LFS_MKTAG(
                            LfsType::Tail as u32 + pdir.split as u32,
                            0x3ff,
                            8,
                        ),
                        buffer: ldir.pair.as_ptr() as *const c_void,
                    },
                ]),
                2,
            );
            lfs_util::lfs_pair_fromle32(ldir.pair.as_mut_ptr());
            if state < 0 {
                return state;
            }

            ldir = pdir.clone();
        }
    }

    if orphans {
        LfsOk::Orphaned as i32
    } else {
        0
    }
}

//  lfs_dir_commit
pub unsafe extern "C" fn lfs_dir_commit(
    lfs: *mut Lfs,
    dir: *mut LfsMdir,
    attrs: *const LfsMattr,
    attrcount: i32,
) -> i32 {
    let orphans = lfs_dir_orphaningcommit(lfs, dir, attrs, attrcount);
    if orphans < 0 {
        return orphans;
    }

    if orphans != 0 {
        let err = lfs_fs_deorphan(lfs, false);
        if err != 0 {
            return err;
        }
    }

    0
}

//  lfs_mkdir_
pub fn lfs_mkdir_(lfs: &mut Lfs, path: *const c_char) -> LfsError {
    // deorphan if we haven't yet, needed at most once after poweron
    let err = unsafe { lfs_fs_forceconsistency(lfs) };
    if err != LfsError::Ok {
        return err;
    }

    let mut cwd = LfsMlist {
        next: lfs.mlist,
        id: 0,
        type_: 0,
        m: LfsMdir {
            pair: [0; 2],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0; 2],
        },
    };

    let mut id: u16 = 0;
    let mut path_ptr = path;
    let err = unsafe { lfs_dir_find(lfs, &mut cwd.m, &mut path_ptr, &mut id) };
    if !(err == LfsError::NoEnt && unsafe { lfs_path_islast(path_ptr) }) {
        return if err < LfsError::Ok { err } else { LfsError::Exist };
    }

    // check that name fits
    let nlen = unsafe { lfs_path_namelen(path_ptr) };
    if nlen as LfsSize > lfs.name_max {
        return LfsError::NameTooLong;
    }

    // build up new directory
    unsafe { lfs_alloc_ckpoint(lfs) };
    let mut dir = LfsMdir::default();
    let err = unsafe { lfs_dir_alloc(lfs, &mut dir) };
    if err != LfsError::Ok {
        return err;
    }

    // find end of list
    let mut pred = cwd.m.clone();
    while pred.split {
        let err = unsafe { lfs_dir_fetch(lfs, &mut pred, pred.tail) };
        if err != LfsError::Ok {
            return err;
        }
    }

    // setup dir
    lfs_pair_tole32(&mut pred.tail);
    let err = unsafe {
        lfs_dir_commit(
            lfs,
            &mut dir,
            &[LfsMattr {
                tag: LfsType::SoftTail as LfsTag | (0x3ff << 16) | (8 << 8),
                buffer: pred.tail.as_mut_ptr() as *const c_void,
            }],
        )
    };
    lfs_pair_fromle32(&mut pred.tail);
    if err != LfsError::Ok {
        return err;
    }

    // current block not end of list?
    if cwd.m.split {
        // update tails, this creates a desync
        let err = unsafe { lfs_fs_preporphans(lfs, 1) };
        if err != LfsError::Ok {
            return err;
        }

        // hook into mlist to catch updates
        cwd.type_ = 0;
        cwd.id = 0;
        let prev_mlist = lfs.mlist;
        lfs.mlist = &mut cwd as *mut LfsMlist;

        lfs_pair_tole32(&mut dir.pair);
        let err = unsafe {
            lfs_dir_commit(
                lfs,
                &mut pred,
                &[LfsMattr {
                    tag: LfsType::SoftTail as LfsTag | (0x3ff << 16) | (8 << 8),
                    buffer: dir.pair.as_mut_ptr() as *const c_void,
                }],
            )
        };
        lfs_pair_fromle32(&mut dir.pair);
        lfs.mlist = prev_mlist;
        if err != LfsError::Ok {
            return err;
        }

        let err = unsafe { lfs_fs_preporphans(lfs, -1) };
        if err != LfsError::Ok {
            return err;
        }
    }

    // now insert into parent block
    lfs_pair_tole32(&mut dir.pair);
    let attrs = [
        LfsMattr {
            tag: LfsType::Create as LfsTag | (id as LfsTag) << 16,
            buffer: std::ptr::null(),
        },
        LfsMattr {
            tag: LfsType::Dir as LfsTag | (id as LfsTag) << 16 | (nlen as LfsTag) << 8,
            buffer: path as *const c_void,
        },
        LfsMattr {
            tag: LfsType::DirStruct as LfsTag | (id as LfsTag) << 16 | 8 << 8,
            buffer: dir.pair.as_ptr() as *const c_void,
        },
        LfsMattr {
            tag: if !cwd.m.split {
                LfsType::SoftTail as LfsTag | (0x3ff << 16) | (8 << 8)
            } else {
                0
            },
            buffer: dir.pair.as_ptr() as *const c_void,
        },
    ];

    let err = unsafe { lfs_dir_commit(lfs, &mut cwd.m, &attrs) };
    lfs_pair_fromle32(&mut dir.pair);
    if err != LfsError::Ok {
        return err;
    }

    LfsError::Ok
}

//  lfs_dir_open_
pub unsafe extern "C" fn lfs_dir_open_(
    lfs: *mut Lfs,
    dir: *mut LfsDir,
    path: *const c_void,
) -> i32 {
    let tag = lfs_dir_find(lfs, &mut (*dir).m, &path, core::ptr::null_mut());
    if tag < 0 {
        return tag as i32;
    }

    if lfs_tag_type3(tag as u32) != LfsType::Dir as u32 {
        return LfsError::NotDir as i32;
    }

    let mut pair: [LfsBlock; 2] = [0; 2];
    if lfs_tag_id(tag as u32) == 0x3ff {
        // 处理根目录
        pair[0] = (*lfs).root[0];
        pair[1] = (*lfs).root[1];
    } else {
        // 从父目录获取块对
        let res = lfs_dir_get(
            lfs,
            &mut (*dir).m,
            LFS_MKTAG(0x700, 0x3ff, 0),
            LFS_MKTAG(LfsType::Struct as u32, lfs_tag_id(tag as u32), 8),
            pair.as_mut_ptr(),
        );
        if res < 0 {
            return res as i32;
        }
        lfs_pair_fromle32(pair.as_mut_ptr());
    }

    // 获取第一个块对
    let err = lfs_dir_fetch(lfs, &mut (*dir).m, pair.as_mut_ptr());
    if err != 0 {
        return err as i32;
    }

    // 初始化目录结构
    (*dir).head[0] = (*dir).m.pair[0];
    (*dir).head[1] = (*dir).m.pair[1];
    (*dir).id = 0;
    (*dir).pos = 0;

    // 添加到元数据链表
    (*dir).type_ = LfsType::Dir as u8;
    lfs_mlist_append(lfs, dir as *mut LfsMlist);

    LfsError::Ok as i32
}

//  lfs_dir_close_
pub unsafe extern "C" fn lfs_dir_close_(lfs: *mut Lfs, dir: *mut LfsDir) -> i32 {
    // remove from list of mdirs
    lfs_mlist_remove(lfs, dir as *mut LfsMlist);

    0
}

//  lfs_dir_read_
pub unsafe extern "C" fn lfs_dir_read_(
    lfs: *mut Lfs,
    dir: *mut LfsDir,
    info: *mut LfsInfo,
) -> i32 {
    // 初始化info结构体
    if !info.is_null() {
        (*info).type_ = 0;
        (*info).size = 0;
        core::ptr::write_bytes((*info).name.as_mut_ptr(), 0, (*info).name.len());
    }

    // 处理特殊偏移量 '.' 和 '..'
    let pos = (*dir).pos;
    if pos == 0 {
        if !info.is_null() {
            (*info).type_ = LfsType::Dir as u8;
            let dot = b".\0";
            core::ptr::copy_nonoverlapping(dot.as_ptr(), (*info).name.as_mut_ptr(), dot.len());
        }
        (*dir).pos += 1;
        return 1;
    } else if pos == 1 {
        if !info.is_null() {
            (*info).type_ = LfsType::Dir as u8;
            let dotdot = b"..\0";
            core::ptr::copy_nonoverlapping(dotdot.as_ptr(), (*info).name.as_mut_ptr(), dotdot.len());
        }
        (*dir).pos += 1;
        return 1;
    }

    loop {
        // 检查是否需要获取新的元数据块
        if (*dir).id == (*dir).m.count as u16 {
            if !(*dir).m.split {
                return 0; // 没有更多条目
            }

            // 获取新的元数据块
            let err = lfs_dir_fetch(lfs, &mut (*dir).m, (*dir).m.tail);
            if err != 0 {
                return err;
            }

            (*dir).id = 0;
        }

        // 获取目录条目信息
        let err = lfs_dir_getinfo(lfs, &mut (*dir).m, (*dir).id, info);
        if err != 0 && err != LfsError::NoEnt as i32 {
            return err;
        }

        (*dir).id += 1;
        if err == 0 {
            break;
        }
    }

    (*dir).pos += 1;
    1
}

//  lfs_dir_seek_
pub(crate) fn lfs_dir_seek_(
    lfs: &mut Lfs,
    dir: &mut LfsDir,
    off: LfsOff,
) -> Result<(), LfsError> {
    // simply walk from head dir
    lfs_dir_rewind_(lfs, dir)?;

    // first two for ./..
    dir.pos = core::cmp::min(2, off);
    let mut remaining_off = off - dir.pos;

    // skip superblock entry
    dir.id = (remaining_off > 0 && lfs_pair_cmp(&dir.head, &lfs.root) == LfsCmp::Eq) as u16;

    while remaining_off > 0 {
        if dir.id == dir.m.count {
            if !dir.m.split {
                return Err(LfsError::Inval);
            }

            lfs_dir_fetch(lfs, &mut dir.m, dir.m.tail)?;
            dir.id = 0;
        }

        let diff = core::cmp::min(
            (dir.m.count - dir.id) as LfsOff,
            remaining_off,
        );
        dir.id += diff as u16;
        dir.pos += diff;
        remaining_off -= diff;
    }

    Ok(())
}

//  lfs_dir_tell_
pub unsafe extern "C" fn lfs_dir_tell_(_lfs: *mut Lfs, dir: *mut LfsDir) -> LfsSoff {
    (*dir).pos
}

//  lfs_dir_rewind_
pub fn lfs_dir_rewind_(lfs: &mut Lfs, dir: &mut LfsDir) -> LfsError {
    // Reload the head dir
    let err = lfs_dir_fetch(lfs, &mut dir.m, dir.head);
    if err != LfsError::Ok {
        return err;
    }

    dir.id = 0;
    dir.pos = 0;
    LfsError::Ok
}

//  lfs_ctz_index
pub fn lfs_ctz_index(lfs: &mut Lfs, off: &mut LfsOff) -> i32 {
    let size = *off;
    let b = unsafe { (*lfs.cfg).block_size } - 8;
    let mut i = size / b;

    if i == 0 {
        return 0;
    }

    i = (size - 4 * (lfs_util::lfs_popc((i - 1) as u32) + 2)) / b;
    *off = size - b * i - 4 * lfs_util::lfs_popc(i as u32);
    i as i32
}

//  lfs_ctz_find
pub fn lfs_ctz_find(
    lfs: &mut Lfs,
    pcache: &LfsCache,
    rcache: &LfsCache,
    mut head: LfsBlock,
    size: LfsSize,
    pos: LfsSize,
    block: &mut LfsBlock,
    off: &mut LfsOff,
) -> LfsError {
    if size == 0 {
        *block = LFS_BLOCK_NULL;
        *off = 0;
        return LfsError::Ok;
    }

    let arg = size.wrapping_sub(1);
    let mut current = lfs_ctz_index(lfs, &arg);
    let pos_arg = pos;
    let target = lfs_ctz_index(lfs, &pos_arg);

    while current > target {
        let skip = lfs_util::lfs_min(
            (lfs_util::lfs_npw2(current - target + 1) - 1) as LfsSize,
            lfs_util::lfs_ctz(current),
        );

        let mut new_head = 0u32;
        let err = lfs_bd_read(
            lfs,
            pcache,
            rcache,
            4,
            head,
            4 * skip,
            &mut new_head as *mut _ as *mut c_void,
            4,
        );

        if err != LfsError::Ok {
            return err;
        }

        new_head = u32::from_le(new_head);
        head = new_head;

        current = current.wrapping_sub(1 << skip);
    }

    *block = head;
    *off = pos;
    LfsError::Ok
}

//  lfs_ctz_extend
pub(crate) fn lfs_ctz_extend(
    lfs: &mut Lfs,
    pcache: &mut LfsCache,
    rcache: &mut LfsCache,
    head: LfsBlock,
    size: LfsSize,
    block: *mut LfsBlock,
    off: *mut LfsOff,
) -> Result<(), LfsError> {
    loop {
        // Allocate new block
        let mut nblock = LfsBlock::default();
        lfs_alloc(lfs, &mut nblock)?;

        // Handle potential relocation
        let res = (|| {
            // Erase the new block
            lfs_bd_erase(lfs, nblock).map_err(|e| {
                if e == LfsError::Corrupt {
                    LfsOk::Relocated
                } else {
                    e
                }
            })?;

            if size == 0 {
                unsafe {
                    *block = nblock;
                    *off = 0;
                }
                return Ok(());
            }

            let mut noff = size - 1;
            let index = lfs_ctz_index(lfs, &mut noff);
            noff += 1;

            // Handle partial block copy
            if noff != lfs.cfg.block_size {
                for i in 0..noff {
                    let mut data = 0u8;
                    lfs_bd_read(
                        lfs,
                        None,
                        rcache,
                        noff - i,
                        head,
                        i,
                        &mut data as *mut _,
                        1,
                    )?;

                    lfs_bd_prog(
                        lfs,
                        Some(pcache),
                        rcache,
                        true,
                        nblock,
                        i,
                        &data as *const _,
                        1,
                    )
                    .map_err(|e| {
                        if e == LfsError::Corrupt {
                            LfsOk::Relocated
                        } else {
                            e
                        }
                    })?;
                }

                unsafe {
                    *block = nblock;
                    *off = noff;
                }
                return Ok(());
            }

            // Append new block
            let index = index + 1;
            let skips = lfs_ctz(index) + 1;
            let mut nhead = head;

            for i in 0..skips {
                nhead = nhead.to_le();
                lfs_bd_prog(
                    lfs,
                    Some(pcache),
                    rcache,
                    true,
                    nblock,
                    4 * i,
                    &nhead as *const _,
                    4,
                )
                .map_err(|e| {
                    if e == LfsError::Corrupt {
                        LfsOk::Relocated
                    } else {
                        e
                    }
                })?;
                nhead = LfsBlock::from_le(nhead);

                if i != skips - 1 {
                    lfs_bd_read(
                        lfs,
                        None,
                        rcache,
                        core::mem::size_of_val(&nhead) as LfsSize,
                        nhead,
                        4 * i,
                        &mut nhead as *mut _,
                        core::mem::size_of_val(&nhead) as LfsSize,
                    )?;
                    nhead = LfsBlock::from_le(nhead);
                }
            }

            unsafe {
                *block = nblock;
                *off = 4 * skips;
            }
            Ok(())
        })();

        match res {
            Ok(()) => return Ok(()),
            Err(LfsError::Corrupt) => {}
            Err(e) => return Err(e),
        }

        // Relocation handling
        lfs_cache_drop(lfs, pcache);
    }
}

//  lfs_file_opencfg_
#[no_mangle]
pub unsafe extern "C" fn lfs_file_opencfg_(
    lfs: *mut lfs,
    file: *mut lfs_file_t,
    path: *const c_char,
    flags: c_int,
    cfg: *const lfs_file_config,
) -> c_int {
    #[cfg(not(feature = "readonly"))]
    {
        if (flags & LFS_O_WRONLY) == LFS_O_WRONLY {
            let err = lfs_fs_forceconsistency(lfs);
            if err != 0 {
                return err;
            }
        }
    }
    #[cfg(feature = "readonly")]
    {
        lfs_assert!((flags & LFS_O_RDONLY) == LFS_O_RDONLY);
    }

    let file = &mut *file;
    let cfg = &*cfg;

    file.cfg = cfg;
    file.flags = flags as u32;
    file.pos = 0;
    file.off = 0;
    file.cache.buffer = ptr::null_mut();

    let mut tag = lfs_dir_find(lfs, &mut file.m, &mut path, &mut file.id);
    if tag < 0 && !(tag == LFS_ERR_NOENT && lfs_path_islast(path)) {
        let err = tag;
        goto cleanup;
    }

    file.type_ = LFS_TYPE_REG;
    lfs_mlist_append(lfs, file as *mut lfs_mlist);

    #[cfg(feature = "readonly")]
    if tag == LFS_ERR_NOENT {
        err = LFS_ERR_NOENT;
        goto cleanup;
    }
    #[cfg(not(feature = "readonly"))]
    {
        if tag == LFS_ERR_NOENT {
            if (flags & LFS_O_CREAT) == 0 {
                err = LFS_ERR_NOENT;
                goto cleanup;
            }

            if lfs_path_isdir(path) {
                err = LFS_ERR_NOTDIR;
                goto cleanup;
            }

            let nlen = lfs_path_namelen(path);
            if nlen > (*lfs).name_max as usize {
                err = LFS_ERR_NAMETOOLONG;
                goto cleanup;
            }

            let mut attrs = [
                lfs_mattr {
                    tag: LFS_MKTAG(LFS_TYPE_CREATE, file.id, 0),
                    buffer: ptr::null(),
                },
                lfs_mattr {
                    tag: LFS_MKTAG(LFS_TYPE_REG, file.id, nlen as u32),
                    buffer: path as *const c_void,
                },
                lfs_mattr {
                    tag: LFS_MKTAG(LFS_TYPE_INLINESTRUCT, file.id, 0),
                    buffer: ptr::null(),
                },
            ];

            let err = lfs_dir_commit(lfs, &mut file.m, &mut attrs);
            if err == LFS_ERR_NOSPC {
                err = LFS_ERR_NAMETOOLONG;
            }
            if err != 0 {
                goto cleanup;
            }

            tag = LFS_MKTAG(LFS_TYPE_INLINESTRUCT, 0, 0);
        } else if (flags & LFS_O_EXCL) != 0 {
            err = LFS_ERR_EXIST;
            goto cleanup;
        }
    }

    if lfs_tag_type3(tag) != LFS_TYPE_REG {
        err = LFS_ERR_ISDIR;
        goto cleanup;
    }

    #[cfg(not(feature = "readonly"))]
    {
        if (flags & LFS_O_TRUNC) != 0 {
            tag = LFS_MKTAG(LFS_TYPE_INLINESTRUCT, file.id, 0);
            file.flags |= LFS_F_DIRTY;
        } else {
            tag = lfs_dir_get(
                lfs,
                &mut file.m,
                LFS_MKTAG(0x700, 0x3ff, 0),
                LFS_MKTAG(LFS_TYPE_STRUCT, file.id, 8),
                &mut file.ctz as *mut _ as *mut c_void,
            );
            if tag < 0 {
                err = tag;
                goto cleanup;
            }
            lfs_ctz_fromle32(&mut file.ctz);
        }
    }

    for i in 0..file.cfg.attr_count as isize {
        let attr = &mut *file.cfg.attrs.offset(i);
        if (file.flags & LFS_O_RDONLY) == LFS_O_RDONLY {
            let res = lfs_dir_get(
                lfs,
                &mut file.m,
                LFS_MKTAG(0x7ff, 0x3ff, 0),
                LFS_MKTAG(LFS_TYPE_USERATTR + attr.type_ as u32, file.id, attr.size),
                attr.buffer,
            );
            if res < 0 && res != LFS_ERR_NOENT {
                err = res;
                goto cleanup;
            }
        }

        #[cfg(not(feature = "readonly"))]
        {
            if (file.flags & LFS_O_WRONLY) == LFS_O_WRONLY {
                if attr.size > (*lfs).attr_max {
                    err = LFS_ERR_NOSPC;
                    goto cleanup;
                }
                file.flags |= LFS_F_DIRTY;
            }
        }
    }

    if !file.cfg.buffer.is_null() {
        file.cache.buffer = file.cfg.buffer as *mut c_void;
    } else {
        file.cache.buffer = lfs_malloc((*lfs).cfg.cache_size as usize);
        if file.cache.buffer.is_null() {
            err = LFS_ERR_NOMEM;
            goto cleanup;
        }
    }

    lfs_cache_zero(lfs, &mut file.cache);

    if lfs_tag_type3(tag) == LFS_TYPE_INLINESTRUCT {
        file.ctz.head = LFS_BLOCK_INLINE;
        file.ctz.size = lfs_tag_size(tag) as u32;
        file.flags |= LFS_F_INLINE;
        file.cache.block = file.ctz.head;
        file.cache.off = 0;
        file.cache.size = (*lfs).cfg.cache_size;

        if file.ctz.size > 0 {
            let res = lfs_dir_get(
                lfs,
                &mut file.m,
                LFS_MKTAG(0x700, 0x3ff, 0),
                LFS_MKTAG(
                    LFS_TYPE_STRUCT,
                    file.id,
                    core::cmp::min(file.cache.size, 0x3fe) as u32,
                ),
                file.cache.buffer,
            );
            if res < 0 {
                err = res;
                goto cleanup;
            }
        }
    }

    return 0;

cleanup:
    #[cfg(not(feature = "readonly"))]
    {
        file.flags |= LFS_F_ERRED;
    }
    lfs_file_close_(lfs, file);
    err
}

#[no_mangle]
pub unsafe extern "C" fn lfs_file_open_(
    lfs: *mut lfs,
    file: *mut lfs_file_t,
    path: *const c_char,
    flags: c_int,
) -> c_int {
    static DEFAULTS: lfs_file_config = lfs_file_config {
        buffer: ptr::null_mut(),
        attrs: ptr::null_mut(),
        attr_count: 0,
    };
    lfs_file_opencfg_(lfs, file, path, flags, &DEFAULTS)
}

#[no_mangle]
pub unsafe extern "C" fn lfs_file_close_(lfs: *mut lfs, file: *mut lfs_file_t) -> c_int {
    let file = &mut *file;
    #[cfg(not(feature = "readonly"))]
    let err = lfs_file_sync_(lfs, file);
    #[cfg(feature = "readonly")]
    let err = 0;

    lfs_mlist_remove(lfs, file as *mut lfs_mlist);

    if file.cfg.buffer.is_null() && !file.cache.buffer.is_null() {
        lfs_free(file.cache.buffer);
        file.cache.buffer = ptr::null_mut();
    }

    err
}

#[cfg(not(feature = "readonly"))]
#[no_mangle]
pub unsafe extern "C" fn lfs_file_relocate(lfs: *mut lfs, file: *mut lfs_file_t) -> c_int {
    loop {
        let mut nblock = 0;
        let mut err = lfs_alloc(lfs, &mut nblock);
        if err != 0 {
            return err;
        }

        err = lfs_bd_erase(lfs, nblock);
        if err == LFS_ERR_CORRUPT {
            continue;
        } else if err != 0 {
            return err;
        }

        for i in 0..file.off as isize {
            let mut data = 0u8;
            if (file.flags & LFS_F_INLINE) != 0 {
                err = lfs_dir_getread(
                    lfs,
                    &mut file.m,
                    ptr::null_mut(),
                    &mut file.cache,
                    file.off - i as u32,
                    LFS_MKTAG(0xfff, 0x1ff, 0),
                    LFS_MKTAG(LFS_TYPE_INLINESTRUCT, file.id, 0),
                    i as u32,
                    &mut data,
                    1,
                );
            } else {
                err = lfs_bd_read(
                    lfs,
                    &mut file.cache,
                    &mut (*lfs).rcache,
                    file.off - i as u32,
                    file.block,
                    i as u32,
                    &mut data,
                    1,
                );
            }
            if err != 0 {
                return err;
            }

            err = lfs_bd_prog(
                lfs,
                &mut (*lfs).pcache,
                &mut (*lfs).rcache,
                true,
                nblock,
                i as u32,
                &data,
                1,
            );
            if err == LFS_ERR_CORRUPT {
                continue;
            } else if err != 0 {
                return err;
            }
        }

        file.cache.block = (*lfs).pcache.block;
        file.cache.off = (*lfs).pcache.off;
        file.cache.size = (*lfs).pcache.size;
        lfs_cache_zero(lfs, &mut (*lfs).pcache);

        file.block = nblock;
        file.flags |= LFS_F_WRITING;
        return 0;
    }
}

#[cfg(not(feature = "readonly"))]
#[no_mangle]
pub unsafe extern "C" fn lfs_file_outline(lfs: *mut lfs, file: *mut lfs_file_t) -> c_int {
    file.off = file.pos;
    lfs_alloc_ckpoint(lfs);
    let err = lfs_file_relocate(lfs, file);
    if err != 0 {
        return err;
    }
    file.flags &= !LFS_F_INLINE;
    0
}

#[no_mangle]
pub unsafe extern "C" fn lfs_file_flush(lfs: *mut lfs, file: *mut lfs_file_t) -> c_int {
    let file = &mut *file;
    if (file.flags & LFS_F_READING) != 0 {
        if (file.flags & LFS_F_INLINE) == 0 {
            lfs_cache_drop(lfs, &mut file.cache);
        }
        file.flags &= !LFS_F_READING;
    }

    #[cfg(not(feature = "readonly"))]
    {
        if (file.flags & LFS_F_WRITING) != 0 {
            let pos = file.pos;
            if (file.flags & LFS_F_INLINE) == 0 {
                // ... rest of flush logic ...
            }

            file.ctz.head = file.block;
            file.ctz.size = file.pos;
            file.flags &= !LFS_F_WRITING;
            file.flags |= LFS_F_DIRTY;
            file.pos = pos;
        }
    }

    0
}

#[cfg(not(feature = "readonly"))]
#[no_mangle]
pub unsafe extern "C" fn lfs_file_sync_(lfs: *mut lfs, file: *mut lfs_file_t) -> c_int {
    let file = &mut *file;
    if (file.flags & LFS_F_ERRED) != 0 {
        return 0;
    }

    let mut err = lfs_file_flush(lfs, file);
    if err != 0 {
        file.flags |= LFS_F_ERRED;
        return err;
    }

    if (file.flags & LFS_F_DIRTY) != 0 && !lfs_pair_isnull(file.m.pair) {
        // ... sync logic ...

        let type_ = if (file.flags & LFS_F_INLINE) != 0 {
            LFS_TYPE_INLINESTRUCT
        } else {
            LFS_TYPE_CTZSTRUCT
        };

        let mut attrs = [
            lfs_mattr {
                tag: LFS_MKTAG(type_, file.id, size),
                buffer: buffer,
            },
            lfs_mattr {
                tag: LFS_MKTAG(LFS_FROM_USERATTRS, file.id, file.cfg.attr_count),
                buffer: file.cfg.attrs as *const c_void,
            },
        ];

        err = lfs_dir_commit(lfs, &mut file.m, &mut attrs);
        if err != 0 {
            file.flags |= LFS_F_ERRED;
            return err;
        }

        file.flags &= !LFS_F_DIRTY;
    }

    0
}

//  lfs_file_open_
#[no_mangle]
pub unsafe extern "C" fn lfs_file_open_(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    path: *const core::ffi::c_char,
    flags: u32,
) -> LfsError {
    let defaults = LfsFileConfig {
        buffer: core::ptr::null_mut(),
        attrs: core::ptr::null_mut(),
        attr_count: 0,
    };
    lfs_file_opencfg_(lfs, file, path, flags, &defaults)
}

//  lfs_file_close_
#[no_mangle]
pub unsafe extern "C" fn lfs_file_close_(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    #[cfg(not(feature = "readonly"))]
    let err = lfs_file_sync_(lfs, file);
    #[cfg(feature = "readonly")]
    let err = 0;

    lfs_mlist_remove(lfs, file as *mut LfsMlist);

    if let Some(cfg) = unsafe { (*file).cfg.as_ref() } {
        if cfg.buffer.is_null() {
            lfs_util::lfs_free((*file).cache.buffer as *mut c_void);
        }
    }

    err
}

//  lfs_file_relocate
static mut LFS_FILE_RELOCATE_COUNT: u32 = 0; // 用于调试，可能需要根据实际实现调整

#[no_mangle]
unsafe extern "C" fn lfs_file_relocate(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let lfs = &mut *lfs;
    let file = &mut *file;

    loop {
        // 分配新块
        let mut nblock: LfsBlock = 0;
        let mut err = lfs_alloc(lfs, &mut nblock);
        if err != 0 {
            return err;
        }

        // 擦除新块
        err = lfs_bd_erase(lfs, nblock);
        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                // 坏块处理
                LFS_DEBUG!("Bad block at 0x{:x}", nblock);
                lfs_cache_drop(lfs, &mut lfs.pcache);
                continue;
            }
            return err;
        }

        // 复制数据到新块
        let mut i: LfsOff = 0;
        while i < file.off {
            let mut data: u8 = 0;

            // 根据是否内联选择读取方式
            if (file.flags & LfsOpenFlags::Inline as u32) != 0 {
                // 内联文件读取
                let tag1 = 0xfff << 20 | 0x1ff << 10 | 0;
                let tag2 = (LfsType::InlineStruct as u32) << 20 | (file.id as u32) << 10 | 0;
                err = lfs_dir_getread(
                    lfs,
                    &mut file.m,
                    core::ptr::null_mut(),
                    &mut file.cache,
                    file.off - i,
                    tag1,
                    tag2,
                    i,
                    &mut data,
                    1,
                );
            } else {
                // 普通块读取
                err = lfs_bd_read(
                    lfs,
                    &mut file.cache,
                    &mut lfs.rcache,
                    file.off - i,
                    file.block,
                    i,
                    &mut data,
                    1,
                );
            }

            if err != 0 {
                return err;
            }

            // 编程数据到新块
            err = lfs_bd_prog(
                lfs,
                &mut lfs.pcache,
                &mut lfs.rcache,
                true,
                nblock,
                i,
                &data,
                1,
            );
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    // 坏块处理
                    LFS_DEBUG!("Bad block at 0x{:x}", nblock);
                    lfs_cache_drop(lfs, &mut lfs.pcache);
                    continue;
                }
                return err;
            }

            i += 1;
        }

        // 同步缓存状态
        let cfg = &*lfs.cfg;
        core::ptr::copy_nonoverlapping(
            lfs.pcache.buffer,
            file.cache.buffer,
            cfg.cache_size as usize,
        );
        file.cache.block = lfs.pcache.block;
        file.cache.off = lfs.pcache.off;
        file.cache.size = lfs.pcache.size;
        lfs_cache_zero(lfs, &mut lfs.pcache);

        // 更新文件元数据
        file.block = nblock;
        file.flags |= LfsOpenFlags::Writing as u32;

        return 0;
    }
}

//  lfs_file_outline
pub fn lfs_file_outline(lfs: &mut Lfs, file: &mut LfsFile) -> LfsError {
    file.off = file.pos;
    lfs_alloc_ckpoint(lfs);
    let err = lfs_file_relocate(lfs, file);
    if err != LfsError::Ok {
        return err;
    }

    file.flags &= !(LfsOpenFlags::Inline as u32);
    LfsError::Ok
}

//  lfs_file_flush
pub fn lfs_file_flush(lfs: &mut Lfs, file: &mut LfsFile) -> LfsSsize {
    // Handle reading flag
    if (file.flags & LfsOpenFlags::Reading as u32) != 0 {
        if (file.flags & LfsOpenFlags::Inline as u32) == 0 {
            lfs_cache_drop(lfs, &mut file.cache);
        }
        file.flags &= !(LfsOpenFlags::Reading as u32);
    }

    #[cfg(not(feature = "readonly"))]
    {
        // Handle writing flag
        if (file.flags & LfsOpenFlags::Writing as u32) != 0 {
            let pos = file.pos;

            if (file.flags & LfsOpenFlags::Inline as u32) == 0 {
                // Create original file copy
                let mut orig = LfsFile {
                    next: core::ptr::null_mut(),
                    id: 0,
                    type_: 0,
                    m: LfsMdir {
                        pair: [0, 0],
                        rev: 0,
                        off: 0,
                        etag: 0,
                        count: 0,
                        erased: false,
                        split: false,
                        tail: [0, 0],
                    },
                    ctz: LfsCtz {
                        head: file.ctz.head,
                        size: file.ctz.size,
                    },
                    flags: LfsOpenFlags::RdOnly as u32,
                    pos: file.pos,
                    block: 0,
                    off: 0,
                    cache: lfs.rcache.clone(),
                    cfg: core::ptr::null(),
                };

                lfs_cache_drop(lfs, &mut lfs.rcache);

                // Copy data block by block
                while file.pos < file.ctz.size {
                    let mut data: u8 = 0;
                    let res = unsafe {
                        lfs_file_flushedread(
                            lfs,
                            &mut orig,
                            &mut data as *mut _ as *mut c_void,
                            1,
                        )
                    };
                    if res < 0 {
                        return res;
                    }

                    let res = unsafe {
                        lfs_file_flushedwrite(
                            lfs,
                            file,
                            &data as *const _ as *const c_void,
                            1,
                        )
                    };
                    if res < 0 {
                        return res;
                    }

                    // Sync cache references
                    if lfs.rcache.block != LFS_BLOCK_NULL {
                        lfs_cache_drop(lfs, &mut orig.cache);
                        lfs_cache_drop(lfs, &mut lfs.rcache);
                    }
                }

                // Flush buffers
                loop {
                    let err = lfs_bd_flush(lfs, &mut file.cache, &mut lfs.rcache, true);
                    if err == 0 {
                        break;
                    } else if err == LfsError::Corrupt as i32 {
                        // Handle relocation
                        let err = lfs_file_relocate(lfs, file);
                        if err != 0 {
                            return err;
                        }
                        continue;
                    } else {
                        return err;
                    }
                }
            } else {
                file.pos = lfs_util::lfs_max(file.pos, file.ctz.size);
            }

            // Update metadata
            file.ctz.head = file.block;
            file.ctz.size = file.pos;
            file.flags &= !(LfsOpenFlags::Writing as u32);
            file.flags |= LfsOpenFlags::Dirty as u32;
            file.pos = pos;
        }
    }

    0
}

//  lfs_file_sync_
pub unsafe fn lfs_file_sync_(lfs: *mut Lfs, file: *mut LfsFile) -> LfsError {
    if (*file).flags & LfsOpenFlags::Erred as u32 != 0 {
        return LfsError::Ok;
    }

    let mut err = lfs_file_flush(lfs, file);
    if err != LfsError::Ok {
        (*file).flags |= LfsOpenFlags::Erred as u32;
        return err;
    }

    if ((*file).flags & LfsOpenFlags::Dirty as u32) != 0
        && !lfs_util::lfs_pair_isnull((*file).m.pair.as_ptr())
    {
        if (*file).flags & LfsOpenFlags::Inline as u32 == 0 {
            err = lfs_bd_sync(lfs, &mut (*lfs).pcache, &mut (*lfs).rcache, false);
            if err != LfsError::Ok {
                return err;
            }
        }

        let type_;
        let buffer;
        let size;
        let mut ctz;
        if (*file).flags & LfsOpenFlags::Inline as u32 != 0 {
            type_ = LfsType::InlineStruct as u16;
            buffer = (*file).cache.buffer as *const c_void;
            size = (*file).ctz.size;
        } else {
            type_ = LfsType::CtzStruct as u16;
            ctz = (*file).ctz.clone();
            lfs_util::lfs_ctz_tole32(&mut ctz);
            buffer = &ctz as *const _ as *const c_void;
            size = core::mem::size_of::<LfsCtz>() as LfsSize;
        }

        let attrs = [
            LfsMattr {
                tag: lfs_util::LFS_MKTAG(type_, (*file).id, size),
                buffer,
            },
            LfsMattr {
                tag: lfs_util::LFS_MKTAG(
                    LfsType::FromUserAttrs as u32,
                    (*file).id,
                    (*(*file).cfg).attr_count,
                ),
                buffer: (*(*file).cfg).attrs as *const c_void,
            },
        ];

        err = lfs_dir_commit(
            lfs,
            &mut (*file).m,
            attrs.as_ptr(),
            attrs.len() as u32,
        );
        if err != LfsError::Ok {
            (*file).flags |= LfsOpenFlags::Erred as u32;
            return err;
        }

        (*file).flags &= !(LfsOpenFlags::Dirty as u32);
    }

    LfsError::Ok
}

//  lfs_file_flushedread
#[no_mangle]
pub unsafe extern "C" fn lfs_file_flushedread(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    buffer: *mut c_void,
    size: LfsSize,
) -> LfsSsize {
    let mut data = buffer as *mut u8;
    let mut nsize = size;

    if file.pos >= file.ctz.size {
        return 0;
    }

    let size = size.min(file.ctz.size - file.pos);
    nsize = size;

    while nsize > 0 {
        if (file.flags & LfsOpenFlags::Reading as u32) == 0
            || file.off == lfs.cfg.block_size
        {
            if (file.flags & LfsOpenFlags::Inline as u32) == 0 {
                let err = lfs_ctz_find(
                    lfs,
                    None,
                    &mut file.cache,
                    file.ctz.head,
                    file.ctz.size,
                    file.pos,
                    &mut file.block,
                    &mut file.off,
                );
                if err != 0 {
                    return err as LfsSsize;
                }
            } else {
                file.block = LFS_BLOCK_INLINE;
                file.off = file.pos;
            }

            file.flags |= LfsOpenFlags::Reading as u32;
        }

        let diff = nsize.min(lfs.cfg.block_size - file.off);
        if (file.flags & LfsOpenFlags::Inline as u32) != 0 {
            let err = lfs_dir_getread(
                lfs,
                &mut file.m,
                None,
                &mut file.cache,
                lfs.cfg.block_size,
                LFS_MKTAG(0xfff, 0x1ff, 0),
                LFS_MKTAG(LfsType::InlineStruct as u32, file.id as u32, 0),
                file.off,
                data as *mut c_void,
                diff,
            );
            if err != 0 {
                return err as LfsSsize;
            }
        } else {
            let err = lfs_bd_read(
                lfs,
                None,
                &mut file.cache,
                lfs.cfg.block_size,
                file.block,
                file.off,
                data as *mut c_void,
                diff,
            );
            if err != 0 {
                return err as LfsSsize;
            }
        }

        file.pos += diff;
        file.off += diff;
        data = data.wrapping_add(diff as usize);
        nsize -= diff;
    }

    size as LfsSsize
}

//  lfs_file_read_
pub unsafe extern "C" fn lfs_file_read_(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    buffer: *mut c_void,
    size: LfsSize,
) -> LfsSsize {
    assert!(((*file).flags & LfsOpenFlags::RdOnly as u32) == LfsOpenFlags::RdOnly as u32);

    if ((*file).flags & LfsOpenFlags::Writing as u32) != 0 {
        let err = lfs_file_flush(lfs, file);
        if err != 0 {
            return err as LfsSsize;
        }
    }

    lfs_file_flushedread(lfs, file, buffer, size)
}

//  lfs_file_flushedwrite
pub unsafe fn lfs_file_flushedwrite(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    buffer: *const c_void,
    size: LfsSize,
) -> LfsSsize {
    let data = buffer as *const u8;
    let mut nsize = size;

    if (file.flags & LfsOpenFlags::Inline as u32) != 0
        && (file.pos + nsize).max(file.ctz.size) > lfs.inline_max
    {
        let err = lfs_file_outline(lfs, file);
        if err != 0 {
            file.flags |= LfsOpenFlags::Erred as u32;
            return err;
        }
    }

    while nsize > 0 {
        if (file.flags & LfsOpenFlags::Writing as u32) == 0 || file.off == lfs.cfg.block_size {
            if (file.flags & LfsOpenFlags::Inline as u32) == 0 {
                if (file.flags & LfsOpenFlags::Writing as u32) == 0 && file.pos > 0 {
                    let mut off: LfsOff = 0;
                    let err = lfs_ctz_find(
                        lfs,
                        None,
                        &mut file.cache,
                        file.ctz.head,
                        file.ctz.size,
                        file.pos - 1,
                        &mut file.block,
                        &mut off,
                    );
                    if err != 0 {
                        file.flags |= LfsOpenFlags::Erred as u32;
                        return err;
                    }
                    lfs_cache_zero(lfs, &mut file.cache);
                }

                lfs_alloc_ckpoint(lfs);
                let err = lfs_ctz_extend(
                    lfs,
                    &mut file.cache,
                    &mut lfs.rcache,
                    file.block,
                    file.pos,
                    &mut file.block,
                    &mut file.off,
                );
                if err != 0 {
                    file.flags |= LfsOpenFlags::Erred as u32;
                    return err;
                }
            } else {
                file.block = LFS_BLOCK_INLINE;
                file.off = file.pos;
            }

            file.flags |= LfsOpenFlags::Writing as u32;
        }

        let diff = nsize.min(lfs.cfg.block_size - file.off);
        loop {
            let err = lfs_bd_prog(
                lfs,
                &mut file.cache,
                &mut lfs.rcache,
                true,
                file.block,
                file.off,
                data,
                diff,
            );
            if err == LfsError::Corrupt as i32 {
                let err = lfs_file_relocate(lfs, file);
                if err != 0 {
                    file.flags |= LfsOpenFlags::Erred as u32;
                    return err;
                }
                continue;
            } else if err != 0 {
                file.flags |= LfsOpenFlags::Erred as u32;
                return err;
            }
            break;
        }

        file.pos += diff;
        file.off += diff;
        data.add(diff as usize);
        nsize -= diff;

        lfs_alloc_ckpoint(lfs);
    }

    size as LfsSsize
}

//  lfs_file_write_
pub fn lfs_file_write_(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    buffer: *const c_void,
    size: LfsSize,
) -> LfsSsize {
    // Check write mode
    if (file.flags & LfsOpenFlags::WrOnly as u32) != LfsOpenFlags::WrOnly as u32 {
        return LfsError::BadF as LfsSsize;
    }

    // Flush if reading
    if (file.flags & LfsOpenFlags::Reading as u32) != 0 {
        let err = lfs_file_flush(lfs, file);
        if err != 0 {
            return err;
        }
    }

    // Handle append mode
    if (file.flags & LfsOpenFlags::Append as u32) != 0 && file.pos < file.ctz.size {
        file.pos = file.ctz.size;
    }

    // Check file size limit
    if file.pos.checked_add(size).map_or(true, |v| v > lfs.file_max) {
        return LfsError::FBig as LfsSsize;
    }

    // Fill gap with zeros
    if (file.flags & LfsOpenFlags::Writing as u32) == 0 && file.pos > file.ctz.size {
        let pos = file.pos;
        file.pos = file.ctz.size;

        while file.pos < pos {
            let zero: u8 = 0;
            let res = lfs_file_flushedwrite(lfs, file, &zero as *const _ as *const c_void, 1);
            if res < 0 {
                return res;
            }
            file.pos += 1;
        }
    }

    // Write actual data
    let nsize = lfs_file_flushedwrite(lfs, file, buffer, size);
    if nsize < 0 {
        return nsize;
    }

    file.flags &= !LfsOpenFlags::Erred as u32;
    nsize
}

//  lfs_file_seek_
static mut lfs_file_seek_: unsafe extern "C" fn(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    off: LfsSoff,
    whence: i32,
) -> LfsSoff = {
    unsafe extern "C" fn lfs_file_seek_(
        lfs: *mut Lfs,
        file: *mut LfsFile,
        off: LfsSoff,
        whence: i32,
    ) -> LfsSoff {
        let file = &mut *file;
        let lfs = &mut *lfs;

        // Calculate new position
        let mut npos = file.pos;
        match whence as u32 {
            LfsWhenceFlags::Set as u32 => npos = off as LfsOff,
            LfsWhenceFlags::Cur as u32 => npos = (file.pos as LfsSoff + off) as LfsOff,
            LfsWhenceFlags::End as u32 => {
                npos = (lfs_file_size_(lfs, file) as LfsSoff + off) as LfsOff
            }
            _ => return LfsError::Inval as LfsSoff,
        }

        // Check position validity
        if npos > lfs.file_max {
            return LfsError::Inval as LfsSoff;
        }

        // Early return if position unchanged
        if file.pos == npos {
            return npos as LfsSoff;
        }

        // Check cache reuse possibility
        if (file.flags & LfsOpenFlags::Reading as u32) != 0
            && file.off != (*lfs).cfg.as_ref().unwrap().block_size
        {
            let opos = file.pos;
            let oindex = lfs_ctz_index(lfs, &opos);
            let mut noff = npos;
            let nindex = lfs_ctz_index(lfs, &mut noff);

            if oindex == nindex
                && noff >= file.cache.off
                && noff < file.cache.off + file.cache.size
            {
                file.pos = npos;
                file.off = noff;
                return npos as LfsSoff;
            }
        }

        // Flush before changing position
        let err = lfs_file_flush(lfs, file);
        if err != 0 {
            return err as LfsSoff;
        }

        // Update position
        file.pos = npos;
        npos as LfsSoff
    }

    lfs_file_seek_
};

//  lfs_file_truncate_
pub(crate) fn lfs_file_truncate_(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    size: LfsOff,
) -> Result<(), LfsError> {
    // 验证文件以写模式打开
    assert!((file.flags & LfsOpenFlags::WrOnly as u32) != 0);

    if size > LFS_FILE_MAX {
        return Err(LfsError::Inval);
    }

    let pos = file.pos;
    let oldsize = lfs_file_size_(lfs, file);

    if size < oldsize {
        if size <= lfs.inline_max {
            // 转换为内联文件
            lfs_file_seek_(lfs, file, 0, LfsWhenceFlags::Set)?;

            // 读取数据到rcache
            lfs_cache_drop(lfs, &mut lfs.rcache);
            lfs_file_flushedread(lfs, file, lfs.rcache.buffer, size)?;

            // 更新文件元数据
            file.ctz.head = LFS_BLOCK_INLINE;
            file.ctz.size = size;
            file.flags |= LfsOpenFlags::Dirty as u32 
                | LfsOpenFlags::Reading as u32 
                | LfsOpenFlags::Inline as u32;
            file.cache.block = file.ctz.head;
            file.cache.off = 0;
            file.cache.size = unsafe { (*lfs.cfg).cache_size };
            
            // 复制缓存数据
            unsafe {
                core::ptr::copy_nonoverlapping(
                    lfs.rcache.buffer,
                    file.cache.buffer,
                    size as usize
                );
            }
        } else {
            // 刷新文件后处理
            lfs_file_flush(lfs, file)?;

            // 查找新的CTZ头
            let mut off: LfsOff = 0;
            lfs_ctz_find(
                lfs,
                None,
                &mut file.cache,
                file.ctz.head,
                file.ctz.size,
                size - 1,
                &mut file.block,
                &mut off,
            )?;

            // 更新文件位置和元数据
            file.pos = size;
            file.ctz.head = file.block;
            file.ctz.size = size;
            file.flags |= LfsOpenFlags::Dirty as u32 | LfsOpenFlags::Reading as u32;
        }
    } else if size > oldsize {
        // 扩展文件并用零填充
        lfs_file_seek_(lfs, file, 0, LfsWhenceFlags::End)?;
        while file.pos < size {
            let zero: u8 = 0;
            lfs_file_write_(lfs, file, &zero, 1)?;
        }
    }

    // 恢复原始文件位置
    lfs_file_seek_(lfs, file, pos, LfsWhenceFlags::Set)?;

    Ok(())
}

//  lfs_file_tell_
pub unsafe fn lfs_file_tell_(_lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    (*file).pos
}

//  lfs_file_rewind_
pub unsafe extern "C" fn lfs_file_rewind_(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let res = lfs_file_seek_(lfs, file, 0, LfsWhenceFlags::Set);
    if res < 0 {
        return res as i32;
    }
    0
}

//  lfs_file_size_
#[inline]
pub unsafe fn lfs_file_size_(lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    let _ = lfs; // 显式忽略未使用的参数
    let file_ref = &*file;

    // 在非只读模式下检查写入标志
    if (file_ref.flags & LfsOpenFlags::Writing as u32) != 0 {
        let pos = file_ref.pos;
        let size = file_ref.ctz.size;
        (pos.max(size)) as LfsSoff
    } else {
        file_ref.ctz.size as LfsSoff
    }
}

//  lfs_stat_
pub(crate) fn lfs_stat_(
    lfs: &mut Lfs,
    path: *mut *const c_char,
    info: &mut LfsInfo,
) -> i32 {
    let mut cwd = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };

    let tag = lfs_dir_find(lfs, &mut cwd, path, ptr::null_mut());

    if tag < 0 {
        return tag as i32;
    }

    // Check for trailing slashes
    unsafe {
        if !(*path).is_null() {
            let remaining = CStr::from_ptr(*path).to_bytes();
            if remaining.contains(&b'/') {
                // Validate directory type
                let type_ = (tag as u32 & 0x700) >> 8;
                if type_ != LfsType::Dir as u32 {
                    return LfsError::NotDir as i32;
                }
            }
        }
    }

    let id = tag as u32 & 0x1fff; // Extract tag ID
    lfs_dir_getinfo(lfs, &cwd, id, info)
}

//  lfs_remove_
#[no_mangle]
pub unsafe extern "C" fn lfs_remove_(lfs: *mut Lfs, path: *const core::ffi::c_char) -> i32 {
    // deorphan if we haven't yet, needed at most once after poweron
    let mut err = lfs_fs_forceconsistency(lfs);
    if err != 0 {
        return err;
    }

    let mut cwd = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };
    let mut path_ptr = path;
    let tag = lfs_dir_find(lfs, &mut cwd, &mut path_ptr, core::ptr::null_mut());
    let id = lfs_tag_id(tag as u32);

    if tag < 0 || id == 0x3ff {
        return if tag < 0 { tag } else { LfsError::Inval as i32 };
    }

    let mut dir = LfsMlist {
        next: (*lfs).mlist,
        id: 0,
        type_: 0,
        m: LfsMdir {
            pair: [0; 2],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0; 2],
        },
    };

    if lfs_tag_type3(tag as u32) == LfsType::Dir as u32 {
        // must be empty before removal
        let mut pair = [0u32; 2];
        let res = lfs_dir_get(
            lfs,
            &mut cwd,
            LFS_MKTAG(0x700, 0x3ff, 0),
            LFS_MKTAG(LfsType::Struct as u32, id, 8),
            pair.as_mut_ptr() as *mut core::ffi::c_void,
        );
        if res < 0 {
            return res as i32;
        }
        lfs_pair_fromle32(pair.as_mut_ptr());

        err = lfs_dir_fetch(lfs, &mut dir.m, pair.as_ptr());
        if err != 0 {
            return err;
        }

        if dir.m.count > 0 || dir.m.split {
            return LfsError::NotEmpty as i32;
        }

        // mark fs as orphaned
        err = lfs_fs_preporphans(lfs, 1);
        if err != 0 {
            return err;
        }

        // I know it's crazy but yes, dir can be changed by our parent's
        // commit (if predecessor is child)
        dir.type_ = 0;
        dir.id = 0;
        (*lfs).mlist = &mut dir as *mut _;
    }

    // delete the entry
    let attrs = [LfsMattr {
        tag: LFS_MKTAG(LfsType::Delete as u32, id, 0),
        buffer: core::ptr::null(),
    }];
    err = lfs_dir_commit(lfs, &mut cwd, attrs.as_ptr(), attrs.len() as i32);
    if err != 0 {
        (*lfs).mlist = dir.next;
        return err;
    }

    (*lfs).mlist = dir.next;
    if lfs_tag_type3(tag as u32) == LfsType::Dir as u32 {
        // fix orphan
        err = lfs_fs_preporphans(lfs, -1);
        if err != 0 {
            return err;
        }

        err = lfs_fs_pred(lfs, dir.m.pair.as_ptr(), &mut cwd);
        if err != 0 {
            return err;
        }

        err = lfs_dir_drop(lfs, &mut cwd, &mut dir.m);
        if err != 0 {
            return err;
        }
    }

    0
}

//  lfs_rename_
pub(crate) unsafe fn lfs_rename_(
    lfs: &mut Lfs,
    oldpath: *const cty::c_char,
    newpath: *const cty::c_char,
) -> cty::c_int {
    // deorphan if we haven't yet, needed at most once after poweron
    let mut err = lfs_fs_forceconsistency(lfs);
    if err != 0 {
        return err;
    }

    // find old entry
    let mut oldcwd = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };
    let oldtag = lfs_dir_find(lfs, &mut oldcwd, &oldpath, core::ptr::null_mut());
    if oldtag < 0 || lfs_tag_id(oldtag as u32) == 0x3ff {
        return if oldtag < 0 { oldtag } else { LfsError::Inval as cty::c_int };
    }

    // find new entry
    let mut newcwd = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };
    let mut newid = 0u16;
    let prevtag = lfs_dir_find(lfs, &mut newcwd, &newpath, &mut newid);
    if (prevtag < 0 || lfs_tag_id(prevtag as u32) == 0x3ff)
        && !(prevtag == LfsError::NoEnt as cty::c_int
            && lfs_path_islast(newpath))
    {
        return if prevtag < 0 {
            prevtag
        } else {
            LfsError::Inval as cty::c_int
        };
    }

    // if we're in the same pair there's a few special cases...
    let samepair = lfs_pair_cmp(oldcwd.pair, newcwd.pair) == LfsCmp::Eq as cty::c_int;
    let mut newoldid = lfs_tag_id(oldtag as u32) as u16;

    let mut prevdir = LfsMlist {
        next: core::ptr::null_mut(),
        id: 0,
        type_: 0,
        m: LfsMdir {
            pair: [0; 2],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0; 2],
        },
    };
    prevdir.next = lfs.mlist;
    if prevtag == LfsError::NoEnt as cty::c_int {
        // if we're a file, don't allow trailing slashes
        if lfs_path_isdir(newpath) && lfs_tag_type3(oldtag as u32) != LfsType::Dir as u32 {
            return LfsError::NotDir as cty::c_int;
        }

        // check that name fits
        let nlen = lfs_path_namelen(newpath);
        if nlen > lfs.name_max as usize {
            return LfsError::NameTooLong as cty::c_int;
        }

        // there is a small chance we are being renamed in the same
        // directory/ to an id less than our old id, the global update
        // to handle this is a bit messy
        if samepair && newid <= newoldid {
            newoldid += 1;
        }
    } else if lfs_tag_type3(prevtag as u32) != lfs_tag_type3(oldtag as u32) {
        return if lfs_tag_type3(prevtag as u32) == LfsType::Dir as u32 {
            LfsError::IsDir as cty::c_int
        } else {
            LfsError::NotDir as cty::c_int
        };
    } else if samepair && newid == newoldid {
        // we're renaming to ourselves??
        return 0;
    } else if lfs_tag_type3(prevtag as u32) == LfsType::Dir as u32 {
        // must be empty before removal
        let mut prevpair = [0; 2];
        let res = lfs_dir_get(
            lfs,
            &mut newcwd,
            LFS_MKTAG(0x700, 0x3ff, 0) as cty::c_int,
            LFS_MKTAG(LfsType::Struct as u32, newid, 8) as cty::c_int,
            prevpair.as_mut_ptr() as *mut cty::c_void,
        );
        if res < 0 {
            return res;
        }
        lfs_pair_fromle32(&mut prevpair);

        // must be empty before removal
        err = lfs_dir_fetch(lfs, &mut prevdir.m, &prevpair);
        if err != 0 {
            return err;
        }

        if prevdir.m.count > 0 || prevdir.m.split {
            return LfsError::NotEmpty as cty::c_int;
        }

        // mark fs as orphaned
        err = lfs_fs_preporphans(lfs, 1);
        if err != 0 {
            return err;
        }

        // I know it's crazy but yes, dir can be changed by our parent's
        // commit (if predecessor is child)
        prevdir.type_ = 0;
        prevdir.id = 0;
        lfs.mlist = &mut prevdir;
    }

    if !samepair {
        lfs_fs_prepmove(lfs, newoldid, oldcwd.pair.as_mut_ptr());
    }

    // move over all attributes
    let attrs = [
        LfsMattr {
            tag: LFS_MKTAG_IF(
                prevtag != LfsError::NoEnt as cty::c_int,
                LfsType::Delete as u32,
                newid,
                0,
            ),
            buffer: core::ptr::null(),
        },
        LfsMattr {
            tag: LFS_MKTAG(LfsType::Create as u32, newid, 0),
            buffer: core::ptr::null(),
        },
        LfsMattr {
            tag: LFS_MKTAG(
                lfs_tag_type3(oldtag as u32),
                newid,
                lfs_path_namelen(newpath) as u32,
            ),
            buffer: newpath as *const cty::c_void,
        },
        LfsMattr {
            tag: LFS_MKTAG(LfsType::FromMove as u32, newid, lfs_tag_id(oldtag as u32)),
            buffer: &oldcwd as *const _ as *const cty::c_void,
        },
        LfsMattr {
            tag: LFS_MKTAG_IF(samepair, LfsType::Delete as u32, newoldid, 0),
            buffer: core::ptr::null(),
        },
    ];
    err = lfs_dir_commit(lfs, &mut newcwd, attrs.as_ptr(), attrs.len() as cty::c_int);
    if err != 0 {
        lfs.mlist = prevdir.next;
        return err;
    }

    // let commit clean up after move (if we're different! otherwise move
    // logic already fixed it for us)
    if !samepair && lfs_gstate_hasmove(&lfs.gstate) {
        // prep gstate and delete move id
        lfs_fs_prepmove(lfs, 0x3ff, core::ptr::null_mut());
        let cleanup_attrs = [LfsMattr {
            tag: LFS_MKTAG(LfsType::Delete as u32, lfs_tag_id(oldtag as u32), 0),
            buffer: core::ptr::null(),
        }];
        err = lfs_dir_commit(lfs, &mut oldcwd, cleanup_attrs.as_ptr(), 1);
        if err != 0 {
            lfs.mlist = prevdir.next;
            return err;
        }
    }

    lfs.mlist = prevdir.next;
    if prevtag != LfsError::NoEnt as cty::c_int
        && lfs_tag_type3(prevtag as u32) == LfsType::Dir as u32
    {
        // fix orphan
        err = lfs_fs_preporphans(lfs, -1);
        if err != 0 {
            return err;
        }

        err = lfs_fs_pred(lfs, prevdir.m.pair.as_mut_ptr(), &mut newcwd);
        if err != 0 {
            return err;
        }

        err = lfs_dir_drop(lfs, &mut newcwd, &mut prevdir.m);
        if err != 0 {
            return err;
        }
    }

    0
}

//  lfs_getattr_
pub fn lfs_getattr_(
    lfs: &mut Lfs,
    path: *const c_char,
    type_: u8,
    buffer: *mut c_void,
    size: LfsSize,
) -> LfsSsize {
    let mut cwd = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };

    let mut path_ptr = path;
    let tag = unsafe { lfs_dir_find(lfs, &mut cwd, &mut path_ptr, core::ptr::null_mut()) };

    if tag < 0 {
        return tag;
    }

    let mut id = lfs_tag_id(tag as LfsTag);
    if id == 0x3ff {
        id = 0;
        let err = lfs_dir_fetch(lfs, &mut cwd, lfs.root);
        if err != LfsError::Ok {
            return err as LfsSsize;
        }
    }

    let tag1 = LFS_MKTAG!(0x7ff, 0x3ff, 0);
    let tag2 = LFS_MKTAG!(
        LfsType::UserAttr as u32 + type_ as u32,
        id,
        size.min(lfs.attr_max)
    );

    let tag = lfs_dir_get(lfs, &mut cwd, tag1, tag2, buffer);

    if tag < 0 {
        if tag == LfsError::NoEnt as LfsSsize {
            return LfsError::NoAttr as LfsSsize;
        }
        return tag;
    }

    lfs_tag_size(tag as LfsTag) as LfsSsize
}

//  lfs_commitattr
pub unsafe extern "C" fn lfs_commitattr(
    lfs: *mut Lfs,
    path: *const c_char,
    type_: u8,
    buffer: *const c_void,
    size: LfsSize,
) -> i32 {
    let mut cwd = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };

    let mut path_ptr = path;
    let tag = lfs_dir_find(
        lfs,
        &mut cwd,
        &mut path_ptr as *mut *const c_char,
        core::ptr::null_mut(),
    );
    if tag < 0 {
        return tag;
    }

    let mut id = lfs_tag_id(tag as u32) as u16;
    if id == 0x3ff {
        id = 0;
        let root = (*lfs).root;
        let err = lfs_dir_fetch(lfs, &mut cwd, root);
        if err != 0 {
            return err;
        }
    }

    let attr_type = LfsType::UserAttr as u32 + type_ as u32;
    let attrs = [LfsMattr {
        tag: lfs_mktag(attr_type, id, size),
        buffer,
    }];

    lfs_dir_commit(lfs, &mut cwd, attrs.as_ptr(), 1)
}

//  lfs_setattr_
pub fn lfs_setattr_(lfs: &mut Lfs, path: *const c_char, type_: u8, buffer: *const c_void, size: LfsSize) -> LfsError {
    if size > lfs.attr_max {
        return LfsError::NoSpc;
    }

    lfs_commitattr(lfs, path, type_, buffer, size)
}

//  lfs_removeattr_
pub unsafe extern "C" fn lfs_removeattr_(
    lfs: *mut Lfs,
    path: *const core::ffi::c_char,
    type_: u8,
) -> i32 {
    lfs_commitattr(lfs, path, type_, core::ptr::null(), 0x3ff)
}

//  lfs_init
pub unsafe extern "C" fn lfs_init(lfs: *mut Lfs, cfg: *const LfsConfig) -> LfsError {
    (*lfs).cfg = cfg;
    (*lfs).block_count = (*cfg).block_count;
    let mut err = LfsError::Ok;

    // LFS_MULTIVERSION断言
    #[cfg(feature = "LFS_MULTIVERSION")]
    {
        let disk_version = (*cfg).disk_version;
        if disk_version != 0 {
            let major = 0xffff & (disk_version >> 16);
            let minor = 0xffff & disk_version;
            assert!(
                major == LFS_DISK_VERSION_MAJOR && minor <= LFS_DISK_VERSION_MINOR,
                "Unsupported disk version"
            );
        }
    }

    // 检查bool类型的真值保留
    assert!((0x80000000i32 != 0), "Invalid bool type");

    // 检查必需的函数指针
    assert!((*cfg).read.is_some(), "Missing read function");
    #[cfg(not(feature = "LFS_READONLY"))]
    {
        assert!((*cfg).prog.is_some(), "Missing prog function");
        assert!((*cfg).erase.is_some(), "Missing erase function");
        assert!((*cfg).sync.is_some(), "Missing sync function");
    }

    // 验证配置大小
    assert!((*cfg).read_size != 0, "Invalid read_size");
    assert!((*cfg).prog_size != 0, "Invalid prog_size");
    assert!((*cfg).cache_size != 0, "Invalid cache_size");

    // 检查缓存对齐
    assert!(
        (*cfg).cache_size % (*cfg).read_size == 0,
        "Cache size must be multiple of read size"
    );
    assert!(
        (*cfg).cache_size % (*cfg).prog_size == 0,
        "Cache size must be multiple of prog size"
    );
    assert!(
        (*cfg).block_size % (*cfg).cache_size == 0,
        "Block size must be multiple of cache size"
    );

    // 检查块大小要求
    assert!((*cfg).block_size >= 128, "Block size too small");
    let divisor = (*cfg).block_size - 2 * 4;
    let value = 0xFFFFFFFFu32 / divisor;
    let npw2 = lfs_util::lfs_npw2(value);
    assert!(4 * npw2 <= (*cfg).block_size, "CTZ math broken");

    // 检查块循环配置
    assert!((*cfg).block_cycles != 0, "Invalid block_cycles");

    // 检查compact_thresh范围
    assert!(
        (*cfg).compact_thresh == 0 || (*cfg).compact_thresh >= (*cfg).block_size / 2,
        "Compact threshold too low"
    );
    assert!(
        (*cfg).compact_thresh == LfsSize::MAX || (*cfg).compact_thresh <= (*cfg).block_size,
        "Compact threshold too high"
    );

    // 检查metadata_max约束
    if (*cfg).metadata_max != 0 {
        assert!(
            (*cfg).metadata_max % (*cfg).read_size == 0,
            "Metadata max must align with read size"
        );
        assert!(
            (*cfg).metadata_max % (*cfg).prog_size == 0,
            "Metadata max must align with prog size"
        );
        assert!(
            (*cfg).block_size % (*cfg).metadata_max == 0,
            "Metadata max must divide block size"
        );
    }

    // 初始化读缓存
    if (*cfg).read_buffer.is_null() {
        (*lfs).rcache.buffer = lfs_malloc((*cfg).cache_size as usize) as *mut u8;
        if (*lfs).rcache.buffer.is_null() {
            err = LfsError::NoMem;
            goto cleanup;
        }
    } else {
        (*lfs).rcache.buffer = (*cfg).read_buffer as *mut u8;
    }

    // 初始化写缓存
    if (*cfg).prog_buffer.is_null() {
        (*lfs).pcache.buffer = lfs_malloc((*cfg).cache_size as usize) as *mut u8;
        if (*lfs).pcache.buffer.is_null() {
            err = LfsError::NoMem;
            goto cleanup;
        }
    } else {
        (*lfs).pcache.buffer = (*cfg).prog_buffer as *mut u8;
    }

    // 初始化缓存
    lfs_cache_zero(lfs, &mut (*lfs).rcache);
    lfs_cache_zero(lfs, &mut (*lfs).pcache);

    // 初始化lookahead缓存
    assert!((*cfg).lookahead_size > 0, "Invalid lookahead size");
    if (*cfg).lookahead_buffer.is_null() {
        (*lfs).lookahead.buffer = lfs_malloc((*cfg).lookahead_size as usize) as *mut u8;
        if (*lfs).lookahead.buffer.is_null() {
            err = LfsError::NoMem;
            goto cleanup;
        }
    } else {
        (*lfs).lookahead.buffer = (*cfg).lookahead_buffer as *mut u8;
    }

    // 设置大小限制
    assert!(
        (*cfg).name_max <= LFS_NAME_MAX as u32,
        "Name max exceeds limit"
    );
    (*lfs).name_max = if (*cfg).name_max != 0 {
        (*cfg).name_max
    } else {
        LFS_NAME_MAX as u32
    };

    assert!((*cfg).file_max <= LFS_FILE_MAX, "File max exceeds limit");
    (*lfs).file_max = if (*cfg).file_max != 0 {
        (*cfg).file_max
    } else {
        LFS_FILE_MAX
    };

    assert!(
        (*cfg).attr_max <= LFS_ATTR_MAX as u32,
        "Attr max exceeds limit"
    );
    (*lfs).attr_max = if (*cfg).attr_max != 0 {
        (*cfg).attr_max
    } else {
        LFS_ATTR_MAX as u32
    };

    // 初始化inline_max
    (*lfs).inline_max = if (*cfg).inline_max == LfsSize::MAX {
        0
    } else if (*cfg).inline_max == 0 {
        let min_val = lfs_util::lfs_min(
            (*cfg).cache_size,
            lfs_util::lfs_min(
                (*lfs).attr_max,
                ((if (*cfg).metadata_max != 0 {
                    (*cfg).metadata_max
                } else {
                    (*cfg).block_size
                }) / 8),
            ),
        );
        min_val
    } else {
        (*cfg).inline_max
    };

    // 初始化状态
    (*lfs).root = [LFS_BLOCK_NULL, LFS_BLOCK_NULL];
    (*lfs).mlist = core::ptr::null_mut();
    (*lfs).seed = 0;
    (*lfs).gdisk = LfsGstate { tag: 0, pair: [0; 2] };
    (*lfs).gstate = LfsGstate { tag: 0, pair: [0; 2] };
    (*lfs).gdelta = LfsGstate { tag: 0, pair: [0; 2] };

    return LfsError::Ok;

cleanup:
    lfs_deinit(lfs);
    err
}

//  lfs_deinit
pub unsafe extern "C" fn lfs_deinit(lfs: *mut Lfs) -> i32 {
    // 释放已分配的内存
    if (*(*lfs).cfg).read_buffer.is_null() {
        lfs_free((*lfs).rcache.buffer as *mut c_void);
    }

    if (*(*lfs).cfg).prog_buffer.is_null() {
        lfs_free((*lfs).pcache.buffer as *mut c_void);
    }

    if (*(*lfs).cfg).lookahead_buffer.is_null() {
        lfs_free((*lfs).lookahead.buffer as *mut c_void);
    }

    0
}

//  lfs_format_
pub fn lfs_format_(lfs: &mut Lfs, cfg: &LfsConfig) -> i32 {
    let err = (|| -> i32 {
        // Initialize filesystem
        let mut err = lfs_init(lfs, cfg);
        if err != 0 {
            return err;
        }

        // Validate block count
        debug_assert!(cfg.block_count != 0);

        // Get config reference
        let lfs_cfg = unsafe { &*lfs.cfg };

        // Initialize lookahead buffer
        unsafe {
            let buffer = core::slice::from_raw_parts_mut(
                lfs.lookahead.buffer,
                lfs_cfg.lookahead_size as usize
            );
            buffer.fill(0);
        }

        // Configure lookahead
        lfs.lookahead.start = 0;
        lfs.lookahead.size = lfs_min(
            8 * lfs_cfg.lookahead_size,
            lfs.block_count
        );
        lfs.lookahead.next = 0;

        // Allocate checkpoint
        err = lfs_alloc_ckpoint(lfs);
        if err != 0 {
            return err;
        }

        // Allocate root directory
        let mut root = LfsMdir::default();
        err = lfs_dir_alloc(lfs, &mut root);
        if err != 0 {
            return err;
        }

        // Prepare superblock
        let superblock = LfsSuperblock {
            version: lfs_fs_disk_version(lfs),
            block_size: lfs_cfg.block_size,
            block_count: lfs.block_count,
            name_max: lfs.name_max,
            file_max: lfs.file_max,
            attr_max: lfs.attr_max,
        };

        // Convert to little-endian
        let mut superblock_le = superblock.clone();
        lfs_superblock_tole32(&mut superblock_le);

        // Commit superblock
        let attrs = [
            LfsMattr {
                tag: lfs_mktag(LfsType::Create, 0, 0),
                buffer: core::ptr::null(),
            },
            LfsMattr {
                tag: lfs_mktag(LfsType::SuperBlock, 0, 8),
                buffer: "littlefs\0".as_ptr() as *const c_void,
            },
            LfsMattr {
                tag: lfs_mktag(LfsType::InlineStruct, 0, core::mem::size_of::<LfsSuperblock>() as u16),
                buffer: &superblock_le as *const _ as *const c_void,
            },
        ];
        err = lfs_dir_commit(lfs, &mut root, &attrs);
        if err != 0 {
            return err;
        }

        // Force compaction
        root.erased = false;
        err = lfs_dir_commit(lfs, &mut root, &[]);
        if err != 0 {
            return err;
        }

        // Verify directory fetch
        let blocks = [0, 1];
        lfs_dir_fetch(lfs, &mut root, &blocks)
    })();

    // Cleanup and return
    lfs_deinit(lfs);
    err
}

// Helper function for minimum calculation
fn lfs_min(a: LfsSize, b: LfsSize) -> LfsSize {
    if a < b { a } else { b }
}

// Helper function for tag creation
fn lfs_mktag(type_: LfsType, id: u16, len: u16) -> LfsTag {
    ((len as u32 & 0x3FF) << 22) | 
    ((id as u32 & 0x3FF) << 12) | 
    (type_ as u32 & 0xFFF)
}

//  lfs_tortoise_detectcycles
pub(crate) fn lfs_tortoise_detectcycles(
    dir: &LfsMdir,
    tortoise: &mut LfsTortoise,
) -> LfsError {
    // Detect cycles with Brent's algorithm
    if lfs_pair_issync(dir.tail, tortoise.pair) {
        // LFS_WARN("Cycle detected in tail list");
        return LfsError::Corrupt;
    }

    if tortoise.i == tortoise.period {
        tortoise.pair[0] = dir.tail[0];
        tortoise.pair[1] = dir.tail[1];
        tortoise.i = 0;
        tortoise.period *= 2;
    }
    tortoise.i += 1;

    LfsError::Ok
}

//  lfs_mount_
pub unsafe extern "C" fn lfs_mount_(lfs: *mut Lfs, cfg: *const LfsConfig) -> LfsError {
    let mut err = lfs_init(lfs, cfg);
    if err != LfsError::Ok {
        return err;
    }

    let mut dir = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0, 1],
    };

    let mut tortoise = LfsTortoise {
        pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
        i: 1,
        period: 1,
    };

    while !lfs_pair_isnull(&dir.tail) {
        err = lfs_tortoise_detectcycles(&mut dir, &mut tortoise);
        if err != LfsError::Ok {
            goto cleanup!();
        }

        let mut find_match = LfsDirFindMatch {
            lfs,
            name: b"littlefs\0".as_ptr() as *const c_void,
            size: 8,
        };

        let tag = lfs_dir_fetchmatch(
            lfs,
            &mut dir,
            dir.tail,
            lfs_mktag(0x7ff, 0x3ff, 0),
            lfs_mktag(LfsType::SuperBlock as u32, 0, 8),
            None,
            Some(lfs_dir_find_match),
            &mut find_match as *mut _ as *mut c_void,
        );

        if tag < 0 {
            err = LfsError::from(tag);
            goto cleanup!();
        }

        if tag != 0 && !lfs_tag_isdelete(tag as LfsTag) {
            (*lfs).root[0] = dir.pair[0];
            (*lfs).root[1] = dir.pair[1];

            let mut superblock = LfsSuperblock::default();
            let tag = lfs_dir_get(
                lfs,
                &mut dir,
                lfs_mktag(0x7ff, 0x3ff, 0),
                lfs_mktag(LfsType::InlineStruct as u32, 0, core::mem::size_of::<LfsSuperblock>() as u32),
                &mut superblock as *mut _ as *mut c_void,
            );

            if tag < 0 {
                err = LfsError::from(tag);
                goto cleanup!();
            }

            lfs_superblock_fromle32(&mut superblock);

            let major_version = (superblock.version >> 16) as u16;
            let minor_version = superblock.version as u16;

            if major_version != lfs_fs_disk_version_major(lfs) as u16 ||
                minor_version > lfs_fs_disk_version_minor(lfs) as u16 
            {
                LFS_ERROR!(
                    "Invalid version v{}.{} != v{}.{}",
                    major_version,
                    minor_version,
                    lfs_fs_disk_version_major(lfs),
                    lfs_fs_disk_version_minor(lfs)
                );
                err = LfsError::Inval;
                goto cleanup!();
            }

            let mut needssuperblock = false;
            if minor_version < lfs_fs_disk_version_minor(lfs) as u16 {
                LFS_DEBUG!(
                    "Found older minor version v{}.{} < v{}.{}",
                    major_version,
                    minor_version,
                    lfs_fs_disk_version_major(lfs),
                    lfs_fs_disk_version_minor(lfs)
                );
                needssuperblock = true;
            }

            lfs_fs_prepsuperblock(lfs, needssuperblock);

            if superblock.name_max != 0 {
                if superblock.name_max > (*lfs).name_max {
                    LFS_ERROR!(
                        "Unsupported name_max (%{} > %{})",
                        superblock.name_max,
                        (*lfs).name_max
                    );
                    err = LfsError::Inval;
                    goto cleanup!();
                }
                (*lfs).name_max = superblock.name_max;
            }

            if superblock.file_max != 0 {
                if superblock.file_max > (*lfs).file_max {
                    LFS_ERROR!(
                        "Unsupported file_max (%{} > %{})",
                        superblock.file_max,
                        (*lfs).file_max
                    );
                    err = LfsError::Inval;
                    goto cleanup!();
                }
                (*lfs).file_max = superblock.file_max;
            }

            if superblock.attr_max != 0 {
                if superblock.attr_max > (*lfs).attr_max {
                    LFS_ERROR!(
                        "Unsupported attr_max (%{} > %{})",
                        superblock.attr_max,
                        (*lfs).attr_max
                    );
                    err = LfsError::Inval;
                    goto cleanup!();
                }
                (*lfs).attr_max = superblock.attr_max;
                (*lfs).inline_max = lfs_min((*lfs).inline_max, (*lfs).attr_max);
            }

            if (*lfs).cfg.as_ref().unwrap().block_count != 0 &&
                superblock.block_count != (*lfs).cfg.as_ref().unwrap().block_count 
            {
                LFS_ERROR!(
                    "Invalid block count (%{} != %{})",
                    superblock.block_count,
                    (*lfs).cfg.as_ref().unwrap().block_count
                );
                err = LfsError::Inval;
                goto cleanup!();
            }

            (*lfs).block_count = superblock.block_count;

            if superblock.block_size != (*lfs).cfg.as_ref().unwrap().block_size {
                LFS_ERROR!(
                    "Invalid block size (%{} != %{})",
                    superblock.block_size,
                    (*lfs).cfg.as_ref().unwrap().block_size
                );
                err = LfsError::Inval;
                goto cleanup!();
            }
        }

        err = lfs_dir_getgstate(lfs, &mut dir, &mut (*lfs).gstate);
        if err != LfsError::Ok {
            goto cleanup!();
        }
    }

    if !lfs_gstate_iszero(&(*lfs).gstate) {
        LFS_DEBUG!(
            "Found pending gstate 0x{:08x}{:08x}{:08x}",
            (*lfs).gstate.tag,
            (*lfs).gstate.pair[0],
            (*lfs).gstate.pair[1]
        );
    }

    (*lfs).gstate.tag += (!lfs_tag_isvalid((*lfs).gstate.tag)) as u32;
    (*lfs).gdisk = (*lfs).gstate.clone();

    (*lfs).lookahead.start = (*lfs).seed % (*lfs).block_count;
    lfs_alloc_drop(lfs);

    return LfsError::Ok;

    cleanup!():
    lfs_unmount_(lfs);
    err
}

// 辅助宏处理跳转逻辑
macro_rules! goto {
    ($label:tt) => {
        break $label;
    };
}

// 清理标签模拟
macro_rules! cleanup {
    () => {
        goto!(cleanup)
    };
}

//  lfs_unmount_
#[no_mangle]
pub unsafe extern "C" fn lfs_unmount_(lfs: *mut Lfs) -> i32 {
    lfs_deinit(lfs)
}

//  lfs_fs_stat_
pub unsafe extern "C" fn lfs_fs_stat_(lfs: *mut Lfs, fsinfo: *mut LfsFsinfo) -> LfsError {
    // Check if superblock is needed
    if !lfs_gstate_needssuperblock(&(*lfs).gstate) {
        (*fsinfo).disk_version = lfs_fs_disk_version(lfs);
    } else {
        // Fetch directory
        let mut dir = LfsMdir {
            pair: [0; 2],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0; 2],
        };

        let err = lfs_dir_fetch(lfs, &mut dir, (*lfs).root);
        if err != 0 {
            return core::mem::transmute(err);
        }

        // Read superblock
        let mut superblock = LfsSuperblock {
            version: 0,
            block_size: 0,
            block_count: 0,
            name_max: 0,
            file_max: 0,
            attr_max: 0,
        };

        let tag1 = (0x7ff << 20) | (0x3ff << 12) | 0; // LFS_MKTAG(0x7ff, 0x3ff, 0)
        let tag2 = ((LfsType::InlineStruct as u32) << 20)
            | (0 << 12)
            | (core::mem::size_of::<LfsSuperblock>() as u32);

        let stag = lfs_dir_get(
            lfs,
            &dir,
            tag1,
            tag2,
            &mut superblock as *mut _ as *mut c_void,
        );

        if stag < 0 {
            return core::mem::transmute(stag);
        }

        lfs_superblock_fromle32(&mut superblock);
        (*fsinfo).disk_version = superblock.version;
    }

    // Populate geometry and configuration
    unsafe {
        (*fsinfo).block_size = (*lfs).cfg.as_ref().unwrap().block_size;
        (*fsinfo).block_count = (*lfs).block_count;
        (*fsinfo).name_max = (*lfs).name_max;
        (*fsinfo).file_max = (*lfs).file_max;
        (*fsinfo).attr_max = (*lfs).attr_max;
    }

    LfsError::Ok
}

//  lfs_fs_traverse_
pub unsafe extern "C" fn lfs_fs_traverse_(
    lfs: *mut Lfs,
    cb: Option<unsafe extern "C" fn(data: *mut c_void, block: LfsBlock) -> i32>,
    data: *mut c_void,
    includeorphans: bool,
) -> i32 {
    let lfs = &mut *lfs;
    let mut dir = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0, 1],
    };

    #[cfg(feature = "migrate")]
    {
        if !lfs.lfs1.is_null() {
            let err = lfs1_traverse(lfs, cb, data);
            if err != 0 {
                return err;
            }
            dir.tail[0] = lfs.root[0];
            dir.tail[1] = lfs.root[1];
        }
    }

    let mut tortoise = LfsTortoise {
        pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
        i: 1,
        period: 1,
    };

    while !lfs_pair_isnull(&dir.tail) {
        let cycle_detected = lfs_tortoise_detectcycles(&dir, &mut tortoise);
        if cycle_detected < 0 {
            return LfsError::Corrupt as i32;
        }

        for i in 0..2 {
            if let Some(callback) = cb {
                let err = callback(data, dir.tail[i]);
                if err != 0 {
                    return err;
                }
            }
        }

        let fetch_result = lfs_dir_fetch(lfs, &mut dir, dir.tail);
        if fetch_result != 0 {
            return fetch_result;
        }

        for id in 0..dir.count as u16 {
            let mut ctz = LfsCtz { head: 0, size: 0 };
            let tag = lfs_dir_get(
                lfs,
                &mut dir,
                LFS_MKTAG(0x700, 0x3ff, 0),
                LFS_MKTAG(LfsType::Struct as u32, id as u32, core::mem::size_of::<LfsCtz>() as u32),
                &mut ctz as *mut _ as *mut c_void,
            );

            if tag < 0 {
                if tag == LfsError::NoEnt as i32 {
                    continue;
                }
                return tag;
            }
            lfs_ctz_fromle32(&mut ctz);

            match lfs_tag_type3(tag as u32) {
                x if x == LfsType::CtzStruct as u32 => {
                    let err = lfs_ctz_traverse(
                        lfs,
                        core::ptr::null_mut(),
                        &mut lfs.rcache,
                        ctz.head,
                        ctz.size,
                        cb,
                        data,
                    );
                    if err != 0 {
                        return err;
                    }
                }
                x if includeorphans && x == LfsType::DirStruct as u32 => {
                    for i in 0..2 {
                        if let Some(callback) = cb {
                            let err = callback(data, ctz.head + i as LfsBlock);
                            if err != 0 {
                                return err;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    #[cfg(not(feature = "readonly"))]
    {
        let mut f = lfs.mlist as *mut LfsFile;
        while !f.is_null() {
            let file = &mut *f;
            if file.type_ != LfsType::Reg as u8 {
                f = file.next;
                continue;
            }

            if (file.flags & LfsOpenFlags::Dirty as u32) != 0
                && (file.flags & LfsOpenFlags::Inline as u32) == 0
            {
                let err = lfs_ctz_traverse(
                    lfs,
                    &mut file.cache,
                    &mut lfs.rcache,
                    file.ctz.head,
                    file.ctz.size,
                    cb,
                    data,
                );
                if err != 0 {
                    return err;
                }
            }

            if (file.flags & LfsOpenFlags::Writing as u32) != 0
                && (file.flags & LfsOpenFlags::Inline as u32) == 0
            {
                let err = lfs_ctz_traverse(
                    lfs,
                    &mut file.cache,
                    &mut lfs.rcache,
                    file.block,
                    file.pos,
                    cb,
                    data,
                );
                if err != 0 {
                    return err;
                }
            }

            f = file.next;
        }
    }

    0
}

#[inline]
fn lfs_pair_isnull(pair: &[LfsBlock; 2]) -> bool {
    pair[0] == LFS_BLOCK_NULL && pair[1] == LFS_BLOCK_NULL
}

#[inline]
fn lfs_tag_type3(tag: u32) -> u32 {
    (tag & 0x00000F00) >> 8
}

//  lfs_fs_pred
pub unsafe extern "C" fn lfs_fs_pred(
    lfs: *mut Lfs,
    pair: [LfsBlock; 2],
    pdir: *mut LfsMdir,
) -> i32 {
    // 初始化目录尾块
    (*pdir).tail = [0, 1];

    // 初始化龟兔算法状态
    let mut tortoise = LfsTortoise {
        pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
        i: 1,
        period: 1,
    };

    // 遍历目录链表
    while !lfs_pair_isnull((*pdir).tail) {
        // 检测循环
        let err = lfs_tortoise_detectcycles(pdir, &mut tortoise);
        if err < 0 {
            return LfsError::Corrupt as i32;
        }

        // 检查是否匹配目标块对
        if lfs_pair_cmp((*pdir).tail, pair) == 0 {
            return 0;
        }

        // 获取下一个目录块
        let err = lfs_dir_fetch(lfs, pdir, (*pdir).tail);
        if err != 0 {
            return err;
        }
    }

    // 未找到目标块对
    LfsError::NoEnt as i32
}

//  lfs_fs_parent_match
#[no_mangle]
pub unsafe extern "C" fn lfs_fs_parent_match(
    data: *mut c_void,
    tag: LfsTag,
    buffer: *const c_void,
) -> i32 {
    let find = &mut *(data as *mut LfsFsParentMatch);
    let lfs = find.lfs;
    let disk = &*(buffer as *const LfsDiskoff);
    
    let mut child: [LfsBlock; 2] = [0; 2];
    let err = lfs_bd_read(
        lfs,
        &mut (*lfs).pcache,
        &mut (*lfs).rcache,
        (*lfs).cfg.as_ref().unwrap().block_size,
        disk.block,
        disk.off,
        child.as_mut_ptr() as *mut c_void,
        core::mem::size_of_val(&child),
    );

    if err != 0 {
        return err;
    }

    lfs_util::lfs_pair_fromle32(&mut child);
    match lfs_util::lfs_pair_cmp(&child, &find.pair) {
        LfsCmp::Eq => LfsCmp::Eq as i32,
        _ => LfsCmp::Lt as i32,
    }
}

//  lfs_fs_parent
pub(crate) static mut LFS_VERSION: u32 = 0x0002000a;
pub(crate) static mut LFS_DISK_VERSION: u32 = 0x00020001;

static LFS_BLOCK_NULL: LfsBlock = !0;

#[inline(always)]
fn lfs_pair_isnull(pair: [LfsBlock; 2]) -> bool {
    pair[0] == LFS_BLOCK_NULL && pair[1] == LFS_BLOCK_NULL
}

pub(crate) unsafe fn lfs_fs_parent(
    lfs: *mut Lfs,
    pair: [LfsBlock; 2],
    parent: *mut LfsMdir,
) -> LfsStag {
    // Initialize parent tail
    (*parent).tail = [0, 1];

    // Initialize tortoise structure
    let mut tortoise = LfsTortoise {
        pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
        i: 1,
        period: 1,
    };

    let mut err = LfsError::Ok as i32;

    // Loop until parent tail becomes null
    while !lfs_pair_isnull((*parent).tail) {
        err = lfs_tortoise_detectcycles(parent, &mut tortoise);
        if err < 0 {
            return err;
        }

        // Prepare match parameters
        let match_param = LfsFsParentMatch {
            lfs,
            pair: [pair[0], pair[1]],
        };

        // Fetch directory with matching parent
        let tag = lfs_dir_fetchmatch(
            lfs,
            parent,
            (*parent).tail,
            LFS_MKTAG(0x7ff, 0, 0x3ff),
            LFS_MKTAG(LfsType::DirStruct as u32, 0, 8),
            None,
            Some(lfs_fs_parent_match),
            &match_param as *const _ as *mut c_void,
        );

        if tag != 0 && tag != LfsError::NoEnt as i32 {
            return tag;
        }
    }

    LfsError::NoEnt as i32
}

// Helper macro for creating tags (assuming existing implementation)
macro_rules! LFS_MKTAG {
    ($type:expr, $id:expr, $size:expr) => {
        ($type << 16) | (($id) << 10) | (($size) & 0x3ff)
    };
}

//  lfs_fs_prepsuperblock
#[no_mangle]
pub unsafe extern "C" fn lfs_fs_prepsuperblock(lfs: &mut Lfs, needssuperblock: bool) {
    lfs.gstate.tag = (lfs.gstate.tag & !0x200) | ((needssuperblock as u32) << 9);
}

//  lfs_fs_preporphans
pub fn lfs_fs_preporphans(&mut self, orphans: i8) -> i32 {
    debug_assert!(
        (self.gstate.tag & 0x1FF) > 0x000 || orphans >= 0,
        "lfs_fs_preporphans assertion failed: tag size > 0x000 or orphans >= 0"
    );
    debug_assert!(
        (self.gstate.tag & 0x1FF) < 0x1FF || orphans <= 0,
        "lfs_fs_preporphans assertion failed: tag size < 0x1FF or orphans <= 0"
    );

    self.gstate.tag = self.gstate.tag.wrapping_add(orphans as i32 as u32);
    self.gstate.tag = (self.gstate.tag & 0x7FFFFFFF)
        | ((lfs_gstate_hasorphans(&self.gstate) as u32) << 31);

    0
}

//  lfs_fs_prepmove
pub unsafe extern "C" fn lfs_fs_prepmove(lfs: *mut Lfs, id: u16, pair: [LfsBlock; 2]) {
    (*lfs).gstate.tag = ((*lfs).gstate.tag & !((0x7ff << 20) | (0x3ff << 10))) |
        ((if id != 0x3ff { (LfsType::Delete as u32) << 20 | (id as u32) << 10 } else { 0 }));
    (*lfs).gstate.pair = if id != 0x3ff { pair } else { [0, 0] };
}

//  lfs_fs_desuperblock
pub fn lfs_fs_desuperblock(lfs: &mut Lfs) -> i32 {
    if !LfsGstate::needssuperblock(&lfs.gstate) {
        return 0;
    }

    log::debug!(
        "Rewriting superblock {{0x{:x}, 0x{:x}}}",
        lfs.root[0],
        lfs.root[1]
    );

    let mut root = LfsMdir::default();
    let err = lfs_dir_fetch(lfs, &mut root, lfs.root);
    if err != 0 {
        return err;
    }

    let mut superblock = LfsSuperblock {
        version: lfs_fs_disk_version(lfs),
        block_size: unsafe { (*lfs.cfg).block_size },
        block_count: lfs.block_count,
        name_max: lfs.name_max,
        file_max: lfs.file_max,
        attr_max: lfs.attr_max,
    };

    lfs_superblock_tole32(&mut superblock);
    let tag = ((LfsType::InlineStruct as u32) << 16) | (0 << 8) | (core::mem::size_of::<LfsSuperblock>() as u32);
    let attrs = [LfsMattr {
        tag,
        buffer: &superblock as *const _ as *const c_void,
    }];

    let err = lfs_dir_commit(lfs, &root, &attrs);
    if err != 0 {
        return err;
    }

    lfs_fs_prepsuperblock(lfs, false);
    0
}

//  lfs_fs_demove
pub(crate) fn lfs_fs_demove(lfs: &mut Lfs) -> Result<(), LfsError> {
    if !LfsGstate::has_move(&lfs.gdisk) {
        return Ok(());
    }

    // Fix bad moves
    #[cfg(feature = "debug")]
    log::debug!(
        "Fixing move {{0x{:x}, 0x{:x}}} 0x{:x}",
        lfs.gdisk.pair[0],
        lfs.gdisk.pair[1],
        lfs_tag_id(lfs.gdisk.tag)
    );

    // Verify delete type in gstate tag
    assert_eq!(
        lfs_tag_type3(lfs.gdisk.tag),
        LfsType::Delete,
        "Unexpected gstate tag type"
    );

    // Fetch and delete the moved entry
    let mut movedir = LfsMdir::default();
    lfs_dir_fetch(lfs, &mut movedir, lfs.gdisk.pair)?;

    // Prepare move state and commit deletion
    let moveid = lfs_tag_id(lfs.gdisk.tag);
    lfs_fs_prepmove(lfs, 0x3ff, None);
    lfs_dir_commit(
        lfs,
        &mut movedir,
        &[LfsMattr {
            tag: (LfsType::Delete as u32) | ((moveid as u32) << 16),
            buffer: core::ptr::null(),
        }],
    )?;

    Ok(())
}

//  lfs_fs_deorphan
pub(crate) fn lfs_fs_deorphan(lfs: &mut Lfs, powerloss: bool) -> Result<i32, LfsError> {
    if !lfs.gstate.has_orphans() {
        return Ok(0);
    }

    let mut pass = 0;
    while pass < 2 {
        let mut pdir = LfsMdir {
            split: true,
            tail: [0, 1],
            pair: [0, 0], // 添加默认值
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
        };
        let mut dir = LfsMdir::default();
        let mut moreorphans = false;

        while !lfs_pair_isnull(pdir.tail) {
            let err = lfs_dir_fetch(lfs, &mut dir, pdir.tail)?;
            if err != 0 {
                return Err(LfsError::from(err));
            }

            if !pdir.split {
                let mut parent = LfsMdir::default();
                let tag = lfs_fs_parent(lfs, pdir.tail, &mut parent)?;

                if pass == 0 && tag != LfsError::NoEnt as i32 {
                    let mut pair = [0, 0];
                    let state = lfs_dir_get(lfs, &parent, LfsTag::MKTAG(0x7ff, 0x3ff, 0), tag, &mut pair)?;
                    if state < 0 {
                        return Ok(state);
                    }
                    lfs_pair_fromle32(&mut pair);

                    if !lfs_pair_issync(&pair, pdir.tail) {
                        let mut moveid = 0x3ff;
                        if lfs.gstate.has_move_here(&pdir.pair) {
                            moveid = lfs_tag_id(lfs.gstate.tag);
                            lfs_fs_prepmove(lfs, 0x3ff, None)?;
                        }

                        lfs_pair_tole32(&mut pair);
                        let state = lfs_dir_orphaningcommit(lfs, &mut pdir, &[
                            LfsMattr { 
                                tag: if moveid != 0x3ff { 
                                    LfsTag::MKTAG(LfsType::Delete as u32, moveid, 0) 
                                } else { 
                                    0 
                                }, 
                                buffer: std::ptr::null() 
                            },
                            LfsMattr { 
                                tag: LfsTag::MKTAG(LfsType::SoftTail as u32, 0x3ff, 8), 
                                buffer: &pair as *const _ as *const c_void 
                            }
                        ])?;
                        lfs_pair_fromle32(&mut pair);

                        if state == LfsOk::Orphaned as i32 {
                            moreorphans = true;
                        }
                        continue;
                    }
                }

                if pass == 1 && tag == LfsError::NoEnt as i32 && powerloss {
                    let err = lfs_dir_getgstate(lfs, &dir, &mut lfs.gdelta)?;
                    if err != 0 {
                        return Err(LfsError::from(err));
                    }

                    lfs_pair_tole32(&mut dir.tail);
                    let state = lfs_dir_orphaningcommit(lfs, &mut pdir, &[LfsMattr {
                        tag: LfsTag::MKTAG(LfsType::Tail as u32 + dir.split as u32, 0x3ff, 8),
                        buffer: &dir.tail as *const _ as *const c_void
                    }])?;
                    lfs_pair_fromle32(&mut dir.tail);

                    if state == LfsOk::Orphaned as i32 {
                        moreorphans = true;
                    }
                    continue;
                }
            }

            pdir = dir.clone();
        }

        pass = if moreorphans { 0 } else { pass + 1 };
    }

    lfs_fs_preporphans(lfs, -(lfs.gstate.get_orphans() as i32))
}

// 辅助函数需要实现（假设在其它模块已定义）
fn lfs_pair_isnull(pair: [LfsBlock; 2]) -> bool {
    pair[0] == LFS_BLOCK_NULL || pair[1] == LFS_BLOCK_NULL
}

fn lfs_pair_fromle32(pair: &mut [LfsBlock; 2]) {
    pair[0] = pair[0].to_le();
    pair[1] = pair[1].to_le();
}

fn lfs_pair_issync(a: &[LfsBlock; 2], b: &[LfsBlock; 2]) -> bool {
    a[0] == b[0] && a[1] == b[1]
}

fn lfs_tag_id(tag: u32) -> u16 {
    (tag & 0x03ff) as u16
}

//  lfs_fs_forceconsistency
pub unsafe extern "C" fn lfs_fs_forceconsistency(lfs: &mut Lfs) -> LfsError {
    let mut err = lfs_fs_desuperblock(lfs);
    if err != LfsError::Ok {
        return err;
    }

    err = lfs_fs_demove(lfs);
    if err != LfsError::Ok {
        return err;
    }

    err = lfs_fs_deorphan(lfs, true);
    if err != LfsError::Ok {
        return err;
    }

    LfsError::Ok
}

//  lfs_fs_mkconsistent_
pub unsafe fn lfs_fs_mkconsistent_(lfs: &mut Lfs) -> Result<(), LfsError> {
    // lfs_fs_forceconsistency does most of the work here
    lfs_fs_forceconsistency(lfs)?;

    // do we have any pending gstate?
    let mut delta = LfsGstate {
        tag: 0,
        pair: [0; 2],
    };
    lfs_gstate_xor(&mut delta, &lfs.gdisk);
    lfs_gstate_xor(&mut delta, &lfs.gstate);

    if !lfs_gstate_iszero(&delta) {
        // lfs_dir_commit will implicitly write out any pending gstate
        let mut root = LfsMdir {
            pair: [0; 2],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0; 2],
        };
        lfs_dir_fetch(lfs, &mut root, lfs.root)?;

        lfs_dir_commit(lfs, &mut root, None, 0)?;
    }

    Ok(())
}

//  lfs_fs_size_count
pub(crate) unsafe extern "C" fn lfs_fs_size_count(p: *mut c_void, _block: LfsBlock) -> i32 {
    let size = &mut *(p as *mut LfsSize);
    *size += 1;
    LfsError::Ok as i32
}

//  lfs_fs_size_
pub(crate) unsafe extern "C" fn lfs_fs_size_(lfs: *mut Lfs) -> LfsSsize {
    let mut size: LfsSize = 0;
    let err = lfs_fs_traverse_(
        lfs,
        Some(lfs_fs_size_count),
        &mut size as *mut LfsSize as *mut c_void,
        false,
    );
    if err != 0 {
        return err as LfsSsize;
    }
    size as LfsSsize
}

unsafe extern "C" fn lfs_fs_size_count(data: *mut c_void, _block: LfsBlock) -> i32 {
    let size = &mut *(data as *mut LfsSize);
    *size += 1;
    0
}

//  lfs_fs_gc_
pub unsafe extern "C" fn lfs_fs_gc_(lfs: *mut Lfs) -> LfsError {
    // Force consistency check
    let mut err = lfs_fs_forceconsistency(lfs);
    if err != LfsError::Ok {
        return err;
    }

    // Check compaction threshold condition
    let cfg = (*lfs).cfg;
    if (*cfg).compact_thresh < (*cfg).block_size - (*cfg).prog_size {
        // Initialize directory iterator with starting tail
        let mut mdir = LfsMdir {
            pair: [0; 2],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0, 1],
        };

        // Iterate through all metadata directories
        while mdir.tail != [LFS_BLOCK_NULL, LFS_BLOCK_NULL] {
            err = lfs_dir_fetch(lfs, &mut mdir, mdir.tail);
            if err != LfsError::Ok {
                return err;
            }

            // Calculate compaction threshold
            let threshold = if (*cfg).compact_thresh == 0 {
                (*cfg).block_size - (*cfg).block_size / 8
            } else {
                (*cfg).compact_thresh
            };

            // Check if needs compaction
            if !mdir.erased || mdir.off > threshold {
                mdir.erased = false;
                err = lfs_dir_commit(lfs, &mut mdir, core::ptr::null(), 0);
                if err != LfsError::Ok {
                    return err;
                }
            }
        }
    }

    // Populate lookahead buffer if needed
    if (*lfs).lookahead.size < 8 * (*cfg).lookahead_size {
        err = lfs_alloc_scan(lfs);
        if err != LfsError::Ok {
            return err;
        }
    }

    LfsError::Ok
}

//  lfs_fs_grow_
pub(crate) fn lfs_fs_grow_(lfs: &mut Lfs, block_count: LfsSize) -> i32 {
    // shrinking is not supported
    assert!(block_count >= lfs.block_count);

    if block_count > lfs.block_count {
        lfs.block_count = block_count;

        // fetch the root
        let mut root = LfsMdir::default();
        let err = lfs_dir_fetch(lfs, &mut root, lfs.root);
        if err != 0 {
            return err;
        }

        // update the superblock
        let mut superblock = LfsSuperblock::default();
        let tag = lfs_dir_get(
            lfs,
            &root,
            lfs_mktag(0x7ff, 0x3ff, 0),
            lfs_mktag(LfsType::InlineStruct as u32, 0, core::mem::size_of_val(&superblock) as u32),
            &mut superblock,
        );
        if tag < 0 {
            return tag;
        }

        lfs_superblock_fromle32(&mut superblock);
        superblock.block_count = lfs.block_count;
        lfs_superblock_tole32(&mut superblock);

        let attrs = [LfsMattr {
            tag: tag as LfsTag,
            buffer: &superblock as *const _ as *const core::ffi::c_void,
        }];
        let err = lfs_dir_commit(lfs, &mut root, &attrs);
        if err != 0 {
            return err;
        }
    }

    0
}

//  lfs1_crc
pub(crate) fn lfs1_crc(crc: &mut u32, buffer: *const c_void, size: usize) {
    *crc = lfs_util::lfs_crc(*crc, buffer, size);
}

//  lfs1_bd_read
#[no_mangle]
pub unsafe extern "C" fn lfs1_bd_read(
    lfs: *mut Lfs,
    block: LfsBlock,
    off: LfsOff,
    buffer: *mut c_void,
    size: LfsSize,
) -> i32 {
    // 如果我们需要处理交替写入对的更多操作，
    // 这里可能需要考虑pcache
    lfs_bd_read(
        &mut *lfs,
        &mut (*lfs).pcache,
        &mut (*lfs).rcache,
        size,
        block,
        off,
        buffer,
        size,
    )
}

//  lfs1_bd_crc
#[no_mangle]
pub unsafe extern "C" fn lfs1_bd_crc(
    lfs: *mut Lfs,
    block: LfsBlock,
    off: LfsOff,
    size: LfsSize,
    crc: *mut u32,
) -> i32 {
    for i in 0..size {
        let mut c: u8 = 0;
        let err = lfs1_bd_read(lfs, block, off + i, &mut c as *mut u8 as *mut c_void, 1);
        if err != 0 {
            return err;
        }

        lfs1_crc(crc, &c as *const u8 as *const c_void, 1);
    }

    0
}

//  lfs1_dir_fromle32
pub(crate) fn lfs1_dir_fromle32(d: &mut LfsMdir) {
    d.rev = lfs_util::lfs_fromle32(d.rev);
    d.tail[0] = lfs_util::lfs_fromle32(d.tail[0]);
    d.tail[1] = lfs_util::lfs_fromle32(d.tail[1]);
}

//  lfs1_dir_tole32
pub fn lfs1_dir_tole32(d: &mut LfsMdir) {
    d.rev = crate::lfs_util::lfs_tole32(d.rev);
    d.tail[0] = crate::lfs_util::lfs_tole32(d.tail[0]);
    d.tail[1] = crate::lfs_util::lfs_tole32(d.tail[1]);
}

//  lfs1_entry_fromle32
pub(crate) fn lfs1_entry_fromle32(d: &mut LfsMdir) {
    d.pair[0] = u32::from_le(d.pair[0]);
    d.pair[1] = u32::from_le(d.pair[1]);
}

//  lfs1_entry_tole32
#[inline]
pub(crate) unsafe fn lfs1_entry_tole32(d: *mut lfs1_disk_entry) {
    (*d).u.dir[0] = lfs_util::lfs_tole32((*d).u.dir[0]);
    (*d).u.dir[1] = lfs_util::lfs_tole32((*d).u.dir[1]);
}

//  lfs1_superblock_fromle32
pub(crate) fn lfs1_superblock_fromle32(d: &mut LfsSuperblock) {
    d.block_size = lfs_util::lfs_fromle32(d.block_size);
    d.block_count = lfs_util::lfs_fromle32(d.block_count);
    d.version = lfs_util::lfs_fromle32(d.version);
}

//  lfs1_entry_size
pub fn lfs1_entry_size(entry: &Lfs1Entry) -> LfsSize {
    4 + entry.d.elen + entry.d.alen + entry.d.nlen
}

//  lfs1_dir_fetch
pub fn lfs1_dir_fetch(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    pair: [LfsBlock; 2],
) -> Result<(), LfsError> {
    let tpair = [pair[0], pair[1]];
    let mut valid = false;

    for i in 0..2 {
        let mut test = Lfs1DiskDir::default();
        let err = unsafe {
            lfs1_bd_read(
                lfs,
                tpair[i],
                0,
                &mut test as *mut _ as *mut c_void,
                core::mem::size_of::<Lfs1DiskDir>() as LfsSize,
            )
        };

        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                continue;
            }
            return Err(unsafe { core::mem::transmute(err) });
        }

        lfs1_dir_fromle32(&mut test);

        if valid && (test.rev < dir.rev) {
            continue;
        }

        let size = test.size & 0x7FFFFFFF;
        if size < (core::mem::size_of::<Lfs1DiskDir>() + 4) as u32 
            || size > lfs.cfg.block_size 
        {
            continue;
        }

        let mut crc = 0xFFFFFFFFu32;
        lfs1_dir_tole32(&mut test);
        lfs1_crc(
            &mut crc,
            &test as *const _ as *const c_void,
            core::mem::size_of_val(&test),
        );
        lfs1_dir_fromle32(&mut test);

        let err = lfs1_bd_crc(
            lfs,
            tpair[i],
            core::mem::size_of::<Lfs1DiskDir>() as LfsSize,
            size - core::mem::size_of::<Lfs1DiskDir>() as u32,
            &mut crc,
        );

        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                continue;
            }
            return Err(unsafe { core::mem::transmute(err) });
        }

        if crc != 0 {
            continue;
        }

        valid = true;
        dir.pair[0] = tpair[(i + 0) % 2];
        dir.pair[1] = tpair[(i + 1) % 2];
        dir.off = core::mem::size_of::<Lfs1DiskDir>() as LfsOff;
        dir.rev = test.rev;
    }

    if !valid {
        return Err(LfsError::Corrupt);
    }

    Ok(())
}

#[derive(Default, Clone, Copy)]
#[repr(C)]
struct Lfs1DiskDir {
    rev: u32,
    size: u32,
    // 其他字段根据实际需要补充
}

fn lfs1_dir_fromle32(dir: &mut Lfs1DiskDir) {
    dir.rev = u32::from_le(dir.rev);
    dir.size = u32::from_le(dir.size);
    // 其他字段转换
}

fn lfs1_dir_tole32(dir: &mut Lfs1DiskDir) {
    dir.rev = dir.rev.to_le();
    dir.size = dir.size.to_le();
    // 其他字段转换
}

//  lfs1_dir_next
pub unsafe extern "C" fn lfs1_dir_next(lfs: *mut Lfs, dir: *mut LfsDir, entry: *mut LfsEntry) -> LfsError {
    loop {
        let dir_m = &mut (*dir).m;
        let dir_off = dir_m.off;
        let dir_size = dir_m.etag; // Assuming etag holds size information with flags
        let size_mask = 0x7FFFFFFF;
        let current_size = (dir_size & size_mask).wrapping_sub(4);

        if dir_off + core::mem::size_of::<EntryD>() as LfsOff <= current_size {
            break;
        }

        if (dir_size & 0x80000000) == 0 {
            (*entry).off = dir_off;
            return LfsError::NoEnt;
        }

        let tail = dir_m.tail;
        let err = lfs1_dir_fetch(lfs, dir, tail);
        if err != LfsError::Ok {
            return err;
        }

        dir_m.off = core::mem::size_of::<LfsMdir>() as LfsOff;
        (*dir).pos += (core::mem::size_of::<LfsMdir>() + 4) as LfsOff;
    }

    let dir_m = &mut (*dir).m;
    let block = dir_m.pair[0];
    let off = dir_m.off;

    let entry_d = &mut (*entry).d as *mut _ as *mut c_void;
    let err = lfs1_bd_read(lfs, block, off, entry_d, core::mem::size_of::<EntryD>());
    if err != LfsError::Ok {
        return err;
    }

    lfs1_entry_fromle32(&mut (*entry).d);

    (*entry).off = off;
    let entry_size = lfs1_entry_size(entry);
    dir_m.off += entry_size;
    (*dir).pos += entry_size;

    LfsError::Ok
}

//  lfs1_moved
pub unsafe extern "C" fn lfs1_moved(lfs: *mut Lfs, e: *const c_void) -> i32 {
    // Check if lfs1's root is null
    let lfs1 = (*lfs).lfs1 as *const Lfs;
    if lfs_util::lfs_pair_isnull((*lfs1).root) {
        return 0;
    }

    // Initialize current working directory
    let mut cwd = LfsMdir {
        pair: [0; 2],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0; 2],
    };

    // Fetch root directory
    let pair = [0 as LfsBlock, 1 as LfsBlock];
    let mut err = lfs1_dir_fetch(lfs, &mut cwd, pair.as_ptr());
    if err != 0 {
        return err;
    }

    // Iterate through directory entries
    while !lfs_util::lfs_pair_isnull(cwd.tail) {
        err = lfs1_dir_fetch(lfs, &mut cwd, cwd.tail.as_ptr());
        if err != 0 {
            return err;
        }

        loop {
            let mut entry = LfsEntry {
                d: LfsEntryData {
                    type_: 0,
                    u: [0; 8], // Adjust size according to actual union size
                }
            };

            err = lfs1_dir_next(lfs, &mut cwd, &mut entry);
            if err != 0 {
                if err == LfsError::NoEnt as i32 {
                    break;
                } else {
                    return err;
                }
            }

            // Check entry type and compare data
            if (entry.d.type_ & 0x80) == 0 {
                let size = core::mem::size_of_val(&entry.d.u);
                let e_slice = core::slice::from_raw_parts(e as *const u8, size);
                let entry_slice = unsafe {
                    core::slice::from_raw_parts(&entry.d.u as *const _ as *const u8, size)
                };

                if e_slice == entry_slice {
                    return 1;
                }
            }
        }
    }

    0
}

//  lfs1_mount
pub unsafe fn lfs1_mount(lfs: &mut Lfs, lfs1: *mut c_void, cfg: &LfsConfig) -> LfsError {
    let mut err = LfsError::Ok;

    // Initialize the filesystem
    err = lfs_init(lfs, cfg);
    if err != LfsError::Ok {
        return err;
    }

    // Setup lfs1 compatibility layer
    lfs.lfs1 = lfs1;
    (*lfs.lfs1.cast::<Lfs>()).root[0] = LFS_BLOCK_NULL;
    (*lfs.lfs1.cast::<Lfs>()).root[1] = LFS_BLOCK_NULL;

    // Initialize free lookahead
    lfs.lookahead.start = 0;
    lfs.lookahead.size = 0;
    lfs.lookahead.next = 0;
    lfs_alloc_ckpoint(lfs);

    // Load superblock
    let mut dir = LfsMdir::default();
    let mut superblock = LfsSuperblock::default();
    let blocks = [0, 1];
    err = lfs1_dir_fetch(lfs, &mut dir, &blocks);
    if err != LfsError::Ok && err != LfsError::Corrupt {
        goto_cleanup!();
    }

    if err == LfsError::Ok {
        // Read superblock data
        err = lfs1_bd_read(
            lfs,
            dir.pair[0],
            core::mem::size_of::<LfsMdir>() as LfsSize,
            &mut superblock as *mut _ as *mut c_void,
            core::mem::size_of::<LfsSuperblock>() as LfsSize,
        );
        lfs1_superblock_fromle32(&mut superblock);
        if err != LfsError::Ok {
            goto_cleanup!();
        }

        // Set root blocks
        (*lfs.lfs1.cast::<Lfs>()).root[0] = superblock.root[0];
        (*lfs.lfs1.cast::<Lfs>()).root[1] = superblock.root[1];
    }

    // Verify superblock magic
    if err != LfsError::Ok || superblock.magic != *b"littlefs" {
        lfs_util::lfs_error("Invalid superblock at {0x0, 0x1}");
        err = LfsError::Corrupt;
        goto_cleanup!();
    }

    // Check version compatibility
    let major_version = (0xffff & (superblock.version >> 16)) as u16;
    let minor_version = (0xffff & (superblock.version >> 0)) as u16;
    if major_version != LFS1_DISK_VERSION_MAJOR || minor_version > LFS1_DISK_VERSION_MINOR {
        lfs_util::lfs_error("Invalid version v{}.{}", major_version, minor_version);
        err = LfsError::Inval;
        goto_cleanup!();
    }

    return LfsError::Ok;

// Cleanup handler
cleanup:
    lfs_deinit(lfs);
    err
}

// Macro for goto-like cleanup handling
macro_rules! goto_cleanup {
    () => {{
        err = err;
        goto cleanup;
    }};
}

//  lfs1_unmount
#[no_mangle]
pub unsafe extern "C" fn lfs1_unmount(lfs: *mut Lfs) -> i32 {
    lfs_deinit(lfs)
}

//  lfs_migrate_
fn lfs_migrate_(lfs: &mut Lfs, cfg: &LfsConfig) -> i32 {
    use core::ptr;

    // Indeterminate filesystem size not allowed for migration.
    assert!(cfg.block_count != 0);

    let mut lfs1 = Lfs1::default();
    let mut err = lfs1_mount(lfs, &mut lfs1, cfg);
    if err != 0 {
        return err;
    }

    {
        let mut dir1 = Lfs1Dir::default();
        let mut dir2 = LfsMdir::default();
        dir1.d.tail = [lfs.lfs1.root[0], lfs.lfs1.root[1]];

        while !lfs_pair_isnull(&dir1.d.tail) {
            err = lfs1_dir_fetch(lfs, &mut dir1, dir1.d.tail);
            if err != 0 {
                goto cleanup;
            }

            err = lfs_dir_alloc(lfs, &mut dir2);
            if err != 0 {
                goto cleanup;
            }

            dir2.rev = dir1.d.rev;
            dir1.head = dir1.pair;
            lfs.root = dir2.pair;

            err = lfs_dir_commit(lfs, &mut dir2, &[]);
            if err != 0 {
                goto cleanup;
            }

            loop {
                let mut entry1 = Lfs1Entry::default();
                err = lfs1_dir_next(lfs, &mut dir1, &mut entry1);
                if err != 0 && err != LfsError::NoEnt as i32 {
                    goto cleanup;
                }
                if err == LfsError::NoEnt as i32 {
                    break;
                }

                if (entry1.d.type_ & 0x80) != 0 {
                    let moved = lfs1_moved(lfs, &entry1.d.u);
                    if moved < 0 {
                        err = moved;
                        goto cleanup;
                    }
                    if moved != 0 {
                        continue;
                    }
                    entry1.d.type_ &= !0x80;
                }

                let mut name = [0u8; LFS_NAME_MAX + 1];
                err = lfs1_bd_read(
                    lfs,
                    dir1.pair[0],
                    entry1.off + 4 + entry1.d.elen + entry1.d.alen,
                    &mut name,
                    entry1.d.nlen,
                );
                if err != 0 {
                    goto cleanup;
                }

                let is_dir = entry1.d.type_ == LFS1_TYPE_DIR;

                err = lfs_dir_fetch(lfs, &mut dir2, &lfs.root);
                if err != 0 {
                    goto cleanup;
                }

                let mut id = 0;
                let name_ptr = name.as_ptr() as *const c_void;
                err = lfs_dir_find(lfs, &mut dir2, &name_ptr, &mut id);
                if !(err == LfsError::NoEnt as i32 && id != 0x3ff) {
                    err = if err < 0 { err } else { LfsError::Exist as i32 };
                    goto cleanup;
                }

                lfs1_entry_tole32(&mut entry1.d);
                let attrs = [
                    LfsMattr {
                        tag: lfs_mktag(LfsType::Create, id, 0),
                        buffer: ptr::null(),
                    },
                    LfsMattr {
                        tag: if is_dir {
                            lfs_mktag(LfsType::Dir, id, entry1.d.nlen)
                        } else {
                            lfs_mktag(LfsType::Reg, id, entry1.d.nlen)
                        },
                        buffer: name.as_ptr() as *const c_void,
                    },
                    LfsMattr {
                        tag: if is_dir {
                            lfs_mktag(LfsType::DirStruct, id, mem::size_of_val(&entry1.d.u) as u32)
                        } else {
                            lfs_mktag(LfsType::CtzStruct, id, mem::size_of_val(&entry1.d.u) as u32)
                        },
                        buffer: &entry1.d.u as *const _ as *const c_void,
                    },
                ];
                lfs1_entry_fromle32(&mut entry1.d);
                err = lfs_dir_commit(lfs, &mut dir2, &attrs);
                if err != 0 {
                    goto cleanup;
                }
            }

            if !lfs_pair_isnull(&dir1.d.tail) {
                err = lfs_dir_fetch(lfs, &mut dir2, &lfs.root);
                if err != 0 {
                    goto cleanup;
                }

                while dir2.split {
                    err = lfs_dir_fetch(lfs, &mut dir2, dir2.tail);
                    if err != 0 {
                        goto cleanup;
                    }
                }

                let tail_attrs = [LfsMattr {
                    tag: lfs_mktag(LfsType::SoftTail, 0x3ff, 8),
                    buffer: &dir1.d.tail as *const _ as *const c_void,
                }];
                err = lfs_dir_commit(lfs, &mut dir2, &tail_attrs);
                if err != 0 {
                    goto cleanup;
                }
            }

            // Copy block data
            err = lfs_bd_erase(lfs, dir1.head[1]);
            if err != 0 {
                goto cleanup;
            }

            err = lfs_dir_fetch(lfs, &mut dir2, &lfs.root);
            if err != 0 {
                goto cleanup;
            }

            for i in 0..dir2.off {
                let mut dat = 0u8;
                err = lfs_bd_read(lfs, None, &lfs.rcache, dir2.off, dir2.pair[0], i, &mut dat, 1);
                if err != 0 {
                    goto cleanup;
                }

                err = lfs_bd_prog(
                    lfs,
                    &mut lfs.pcache,
                    &mut lfs.rcache,
                    true,
                    dir1.head[1],
                    i,
                    &dat,
                    1,
                );
                if err != 0 {
                    goto cleanup;
                }
            }

            err = lfs_bd_flush(lfs, &mut lfs.pcache, &mut lfs.rcache, true);
            if err != 0 {
                goto cleanup;
            }
        }

        // Create new superblock
        err = lfs1_dir_fetch(lfs, &mut dir1, &[0, 1]);
        if err != 0 {
            goto cleanup;
        }

        dir2.pair = dir1.pair;
        dir2.rev = dir1.d.rev;
        dir2.off = mem::size_of_val(&dir2.rev) as LfsOff;
        dir2.etag = 0xffffffff;
        dir2.count = 0;
        dir2.tail = lfs.lfs1.root;
        dir2.erased = false;
        dir2.split = true;

        let mut superblock = LfsSuperblock {
            version: LFS_DISK_VERSION,
            block_size: lfs.cfg.block_size,
            block_count: lfs.cfg.block_count,
            name_max: lfs.name_max,
            file_max: lfs.file_max,
            attr_max: lfs.attr_max,
        };
        lfs_superblock_tole32(&mut superblock);

        let super_attrs = [
            LfsMattr {
                tag: lfs_mktag(LfsType::Create, 0, 0),
                buffer: ptr::null(),
            },
            LfsMattr {
                tag: lfs_mktag(LfsType::SuperBlock, 0, 8),
                buffer: b"littlefs\0".as_ptr() as *const c_void,
            },
            LfsMattr {
                tag: lfs_mktag(LfsType::InlineStruct, 0, mem::size_of_val(&superblock) as u32),
                buffer: &superblock as *const _ as *const c_void,
            },
        ];
        err = lfs_dir_commit(lfs, &mut dir2, &super_attrs);
        if err != 0 {
            goto cleanup;
        }

        // Force compaction
        dir2.erased = false;
        err = lfs_dir_commit(lfs, &mut dir2, &[]);
        if err != 0 {
            goto cleanup;
        }
    }

cleanup:
    lfs1_unmount(lfs);
    err
}

// Helper functions
fn lfs_pair_isnull(pair: &[LfsBlock; 2]) -> bool {
    pair[0] == LFS_BLOCK_NULL && pair[1] == LFS_BLOCK_NULL
}

fn lfs_mktag(type_: LfsType, id: u16, len: u32) -> u32 {
    ((type_ as u32) << 16) | ((id as u32) << 8) | (len & 0xff)
}

//  lfs_format
pub unsafe extern "C" fn lfs_format(lfs: *mut Lfs, cfg: *const LfsConfig) -> i32 {
    let cfg_ref = &*cfg;
    
    // LFS_LOCK
    let mut err = if let Some(lock_fn) = cfg_ref.lock {
        lock_fn(cfg)
    } else {
        0
    };
    if err != 0 {
        return err;
    }

    // LFS_TRACE
    lfs_util::trace!(
        "lfs_format({:p}, {:p} {{.context={:p}, .read={:p}, .prog={:p}, .erase={:p}, .sync={:p}, \
        .read_size={}, .prog_size={}, .block_size={}, .block_count={}, .block_cycles={}, \
        .cache_size={}, .lookahead_size={}, .read_buffer={:p}, .prog_buffer={:p}, \
        .lookahead_buffer={:p}, .name_max={}, .file_max={}, .attr_max={}})",
        lfs as *mut c_void,
        cfg as *mut c_void,
        cfg_ref.context,
        cfg_ref.read.map(|f| f as *const c_void).unwrap_or(ptr::null()),
        cfg_ref.prog.map(|f| f as *const c_void).unwrap_or(ptr::null()),
        cfg_ref.erase.map(|f| f as *const c_void).unwrap_or(ptr::null()),
        cfg_ref.sync.map(|f| f as *const c_void).unwrap_or(ptr::null()),
        cfg_ref.read_size,
        cfg_ref.prog_size,
        cfg_ref.block_size,
        cfg_ref.block_count,
        cfg_ref.block_cycles,
        cfg_ref.cache_size,
        cfg_ref.lookahead_size,
        cfg_ref.read_buffer,
        cfg_ref.prog_buffer,
        cfg_ref.lookahead_buffer,
        cfg_ref.name_max,
        cfg_ref.file_max,
        cfg_ref.attr_max
    );

    // Call actual format implementation
    err = lfs_format_(lfs, cfg);

    lfs_util::trace!("lfs_format -> {}", err);

    // LFS_UNLOCK
    if let Some(unlock_fn) = cfg_ref.unlock {
        unlock_fn(cfg);
    }

    err
}

//  lfs_mount
pub unsafe extern "C" fn lfs_mount(lfs: *mut Lfs, cfg: *const LfsConfig) -> i32 {
    use core::ptr;
    use core::mem::transmute;

    // Lock the configuration
    let err = (*cfg).lock.map_or(0, |f| f(cfg));
    if err != 0 {
        return err;
    }

    // Trace mount parameters
    lfs_util::trace!(
        "lfs_mount({:p}, {:p} {{.context={:p}, .read={:p}, .prog={:p}, .erase={:p}, .sync={:p}, \
        .read_size={}, .prog_size={}, .block_size={}, .block_count={}, .block_cycles={}, \
        .cache_size={}, .lookahead_size={}, .read_buffer={:p}, .prog_buffer={:p}, \
        .lookahead_buffer={:p}, .name_max={}, .file_max={}, .attr_max={}}})",
        lfs,
        cfg,
        (*cfg).context,
        (*cfg).read.map_or(ptr::null(), |f| transmute(f)),
        (*cfg).prog.map_or(ptr::null(), |f| transmute(f)),
        (*cfg).erase.map_or(ptr::null(), |f| transmute(f)),
        (*cfg).sync.map_or(ptr::null(), |f| transmute(f)),
        (*cfg).read_size,
        (*cfg).prog_size,
        (*cfg).block_size,
        (*cfg).block_count,
        (*cfg).block_cycles,
        (*cfg).cache_size,
        (*cfg).lookahead_size,
        (*cfg).read_buffer,
        (*cfg).prog_buffer,
        (*cfg).lookahead_buffer,
        (*cfg).name_max,
        (*cfg).file_max,
        (*cfg).attr_max
    );

    // Perform actual mount operation
    let err = lfs_mount_(lfs, cfg);

    // Trace result and unlock
    lfs_util::trace!("lfs_mount -> {}", err);
    (*cfg).unlock.map(|f| f(cfg));
    err
}

//  lfs_unmount
pub unsafe extern "C" fn lfs_unmount(lfs: *mut Lfs) -> i32 {
    let lfs = &mut *lfs;
    let cfg = &*lfs.cfg;

    // 获取锁
    let err = cfg.lock.map_or(0, |f| f(cfg as *const _));
    if err != 0 {
        return err;
    }

    // 追踪调用
    LFS_TRACE!("lfs_unmount({:p})", lfs);

    // 实际卸载操作
    let err = lfs_unmount_(lfs);

    // 追踪结果
    LFS_TRACE!("lfs_unmount -> {}", err);

    // 释放锁
    cfg.unlock.map(|f| f(cfg as *const _));

    err
}

//  lfs_remove
pub unsafe extern "C" fn lfs_remove(lfs: *mut Lfs, path: *const c_char) -> i32 {
    let cfg = &(*lfs).cfg;
    let err = match (*cfg).lock {
        Some(lock) => lock(cfg),
        None => LfsError::Io as i32,
    };
    if err != 0 {
        return err;
    }

    // LFS_TRACE宏在Rust中通常用log crate实现，这里保持原样
    // 实际实现可能需要使用log::trace!或类似机制
    // log::trace!("lfs_remove({:p}, \"{}\")", lfs, CStr::from_ptr(path).to_str().unwrap());

    let err = lfs_remove_(lfs, path);

    // log::trace!("lfs_remove -> {}", err);

    match (*cfg).unlock {
        Some(unlock) => unlock(cfg),
        None => LfsError::Io as i32,
    };

    err
}

//  lfs_rename
pub fn lfs_rename(lfs: &mut Lfs, oldpath: *const c_char, newpath: *const c_char) -> i32 {
    let cfg = unsafe { &*lfs.cfg };
    let err = if let Some(lock) = cfg.lock {
        unsafe { lock(cfg) }
    } else {
        LfsError::Ok as i32
    };
    
    if err != LfsError::Ok as i32 {
        return err;
    }

    lfs_util::trace!("lfs_rename({:p}, \"{}\", \"{}\")", 
        lfs as *const _ as *mut c_void,
        unsafe { CStr::from_ptr(oldpath).to_string_lossy() },
        unsafe { CStr::from_ptr(newpath).to_string_lossy() });

    let err = unsafe { lfs_rename_(lfs, oldpath, newpath) };

    lfs_util::trace!("lfs_rename -> {}", err);

    if let Some(unlock) = cfg.unlock {
        unsafe { unlock(cfg) };
    }

    err
}

//  lfs_stat
pub unsafe extern "C" fn lfs_stat(lfs: *mut Lfs, path: *const c_char, info: *mut LfsInfo) -> i32 {
    use core::ffi::CStr;

    let cfg = (*lfs).cfg;
    let lock_err = if let Some(lock_fn) = (*cfg).lock {
        lock_fn(cfg)
    } else {
        0
    };
    if lock_err != 0 {
        return lock_err;
    }

    let path_str = if path.is_null() {
        "(null)"
    } else {
        match CStr::from_ptr(path).to_str() {
            Ok(s) => s,
            Err(_) => "(invalid)",
        }
    };
    LFS_TRACE!("lfs_stat({:p}, \"{}\", {:p})", lfs, path_str, info);

    let err = lfs_stat_(lfs, path, info);

    LFS_TRACE!("lfs_stat -> {}", err);

    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }

    err
}

//  lfs_getattr
#[no_mangle]
pub unsafe extern "C" fn lfs_getattr(
    lfs: *mut Lfs,
    path: *const c_char,
    type_: u8,
    buffer: *mut c_void,
    size: LfsSize,
) -> LfsSsize {
    let cfg = &(*lfs).cfg;
    let lock = (*cfg).lock.unwrap();
    let err = lock(*cfg);
    if err != 0 {
        return err as LfsSsize;
    }

    LFS_TRACE!(
        "lfs_getattr({:p}, \"{:?}\", {}, {:p}, {})",
        lfs,
        path,
        type_,
        buffer,
        size
    );

    let res = lfs_getattr_(lfs, path, type_, buffer, size);

    LFS_TRACE!("lfs_getattr -> {}", res);

    let unlock = (*cfg).unlock.unwrap();
    unlock(*cfg);
    res
}

//  lfs_setattr
pub unsafe extern "C" fn lfs_setattr(
    lfs: *mut Lfs,
    path: *const c_char,
    type_: u8,
    buffer: *const c_void,
    size: LfsSize,
) -> i32 {
    // 获取配置引用
    let cfg = &*((*lfs).cfg);
    
    // 尝试加锁
    let lock_err = if let Some(lock_fn) = cfg.lock {
        lock_fn(cfg as *const LfsConfig)
    } else {
        0 // 无锁实现直接返回成功
    };
    if lock_err != 0 {
        return lock_err;
    }

    // 转换路径为可打印字符串（调试用）
    let path_str = if !path.is_null() {
        core::ffi::CStr::from_ptr(path).to_string_lossy()
    } else {
        core::borrow::Cow::Borrowed("(null)")
    };

    // 记录调试跟踪信息
    lfs_util::lfs_trace!(
        "lfs_setattr({:p}, \"{}\", {}, {:p}, {})",
        lfs,
        path_str,
        type_,
        buffer,
        size
    );

    // 调用核心实现
    let err = lfs_setattr_(lfs, path, type_, buffer, size);

    // 记录返回结果
    lfs_util::lfs_trace!("lfs_setattr -> {}", err);

    // 解锁操作
    if let Some(unlock_fn) = cfg.unlock {
        unlock_fn(cfg as *const LfsConfig);
    }

    err
}

//  lfs_removeattr
pub unsafe extern "C" fn lfs_removeattr(lfs: *mut Lfs, path: *const c_char, type_: u8) -> i32 {
    let cfg = (*lfs).cfg;
    let lock = (*cfg).lock.unwrap();
    let err = lock(cfg);
    if err != 0 {
        return err;
    }

    lfs_util::trace!(
        "lfs_removeattr({:p}, \"{}\", {})",
        lfs,
        core::ffi::CStr::from_ptr(path).to_str().unwrap(),
        type_
    );

    let err = lfs_removeattr_(lfs, path, type_);

    lfs_util::trace!("lfs_removeattr -> {}", err);
    
    let unlock = (*cfg).unlock.unwrap();
    unlock(cfg);
    
    err
}

//  lfs_file_open
pub fn lfs_file_open(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    path: *const core::ffi::c_char,
    flags: u32,
) -> i32 {
    let cfg = lfs.cfg;
    // 锁定设备
    let lock_err = unsafe { (*cfg).lock.map(|f| f(cfg)).unwrap_or(0) };
    if lock_err != 0 {
        return lock_err;
    }

    // 跟踪调试信息
    lfs_util::trace!(
        "lfs_file_open({:p}, {:p}, \"{}\", {:x})",
        lfs as *const _,
        file as *const _,
        unsafe { core::ffi::CStr::from_ptr(path) }.to_str().unwrap(),
        flags
    );

    // 断言检查文件未在挂载列表中打开
    assert!(!lfs_mlist_isopen(
        lfs.mlist,
        file as *mut LfsFile as *mut LfsMlist
    ));

    // 调用核心文件打开逻辑
    let err = lfs_file_open_(lfs, file, path, flags);

    // 跟踪返回结果
    lfs_util::trace!("lfs_file_open -> {}", err);

    // 解锁设备
    unsafe { (*cfg).unlock.map(|f| f(cfg)) };

    err
}

//  lfs_file_opencfg
pub unsafe extern "C" fn lfs_file_opencfg(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    path: *const core::ffi::c_char,
    flags: u32,
    cfg: *const LfsFileConfig,
) -> i32 {
    use core::ffi::CStr;

    // LOCK操作
    let cfg_ptr = (*lfs).cfg;
    let lock_err = if let Some(lock_fn) = (*cfg_ptr).lock {
        lock_fn(cfg_ptr)
    } else {
        0
    };
    if lock_err != 0 {
        return lock_err;
    }

    // TRACE日志
    let path_str = CStr::from_ptr(path).to_string_lossy();
    let cfg_attrs_count = if cfg.is_null() { 0 } else { (*cfg).attr_count };
    let _ = lfs_util::trace!(
        "lfs_file_opencfg({:p}, {:p}, \"{}\", {:x}, {:p} {{ .buffer={:p}, .attrs={:p}, .attr_count={} }})",
        lfs,
        file,
        path_str,
        flags,
        cfg,
        if cfg.is_null() { core::ptr::null() } else { (*cfg).buffer },
        if cfg.is_null() { core::ptr::null() } else { (*cfg).attrs },
        cfg_attrs_count
    );

    // ASSERT检查
    assert!(
        !lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist),
        "File already open in mlist"
    );

    // 核心操作
    let err = lfs_file_opencfg_(lfs, file, path, flags, cfg);

    // TRACE结果
    let _ = lfs_util::trace!("lfs_file_opencfg -> {}", err);

    // UNLOCK操作
    if let Some(unlock_fn) = (*cfg_ptr).unlock {
        unlock_fn(cfg_ptr);
    }

    err
}

//  lfs_file_close
pub unsafe extern "C" fn lfs_file_close(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let lfs_ref = &mut *lfs;
    let cfg = &*lfs_ref.cfg;

    // Acquire lock
    let err = match cfg.lock {
        Some(lock_fn) => lock_fn(cfg as *const _),
        None => 0,
    };
    if err != 0 {
        return err;
    }

    // Log trace
    lfs_util::lfs_trace!("lfs_file_close({:p}, {:p})", lfs, file);

    // Verify file is in open list
    let mlist_ptr = file as *mut LfsMlist;
    assert!(lfs_mlist_isopen(lfs_ref.mlist, mlist_ptr));

    // Perform actual close operation
    let err = lfs_file_close_(lfs, file);

    // Log result
    lfs_util::lfs_trace!("lfs_file_close -> {}", err);

    // Release lock
    if let Some(unlock_fn) = cfg.unlock {
        unlock_fn(cfg as *const _);
    }

    err
}

//  lfs_file_sync
pub unsafe extern "C" fn lfs_file_sync(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let cfg = (*lfs).cfg;
    
    // Lock
    let err = if let Some(lock_fn) = (*cfg).lock {
        lock_fn(cfg)
    } else {
        0
    };
    if err != 0 {
        return err;
    }
    
    // Trace
    lfs_util::lfs_trace!("lfs_file_sync({:p}, {:p})", lfs as *mut c_void, file as *mut c_void);
    
    // Assert
    debug_assert!(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));
    
    // Call implementation
    let err = lfs_file_sync_(lfs, file);
    
    // Trace result
    lfs_util::lfs_trace!("lfs_file_sync -> {}", err);
    
    // Unlock
    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }
    
    err
}

//  lfs_file_read
pub unsafe extern "C" fn lfs_file_read(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    buffer: *mut c_void,
    size: LfsSize,
) -> LfsSsize {
    let cfg = (*lfs).cfg;
    let lock_fn = (*cfg).lock.expect("lock function must be set");
    let err = lock_fn(cfg);
    if err != 0 {
        return err as LfsSsize;
    }

    // LFS_TRACE would typically be implemented with logging here
    // LFS_ASSERT is converted to Rust's assert!
    assert!(lfs_mlist_isopen(
        (*lfs).mlist,
        file as *mut LfsMlist as *mut c_void
    ));

    let res = lfs_file_read_(lfs, file, buffer, size);

    // LFS_TRACE for result would be here
    let unlock_fn = (*cfg).unlock.expect("unlock function must be set");
    unlock_fn(cfg);

    res
}

//  lfs_file_write
pub unsafe extern "C" fn lfs_file_write(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    buffer: *const c_void,
    size: LfsSize,
) -> LfsSsize {
    let cfg = (*lfs).cfg;
    let err = if let Some(lock_fn) = (*cfg).lock {
        lock_fn(cfg)
    } else {
        0
    };
    if err != 0 {
        return err as LfsSsize;
    }

    lfs_util::trace!(
        "lfs_file_write({:p}, {:p}, {:p}, {})",
        lfs as *mut _,
        file as *mut _,
        buffer,
        size
    );

    assert!(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

    let res = lfs_file_write_(lfs, file, buffer, size);

    lfs_util::trace!("lfs_file_write -> {}", res);

    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }

    res
}

//  lfs_file_seek
#[no_mangle]
pub unsafe extern "C" fn lfs_file_seek(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    off: LfsSoff,
    whence: LfsWhenceFlags,
) -> LfsSoff {
    let cfg = (*lfs).cfg;
    let err = (*cfg).lock.map_or(0, |f| f(cfg));
    if err != 0 {
        return err as LfsSoff;
    }

    // LFS_TRACE宏的Rust实现可能需要使用日志库，此处保留原始调用格式
    // LFS_TRACE!("lfs_file_seek({:p}, {:p}, {}, {})", lfs, file, off, whence as i32);

    // 原始C断言转换为Rust的debug_assert!
    // debug_assert!(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

    let res = lfs_file_seek_(lfs, file, off, whence);

    // LFS_TRACE!("lfs_file_seek -> {}", res);
    // LFS_UNLOCK宏对应解锁操作
    (*cfg).unlock.map(|f| f(cfg));

    res
}

//  lfs_file_truncate
pub unsafe extern "C" fn lfs_file_truncate(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    size: LfsOff,
) -> i32 {
    // Acquire lock
    let cfg = &*(*lfs).cfg;
    let lock_err = if let Some(lock_fn) = cfg.lock {
        lock_fn(cfg as *const LfsConfig)
    } else {
        0 // Assume locking is not required if no lock function
    };
    if lock_err != 0 {
        return lock_err;
    }

    // Create guard to ensure unlock happens
    struct Guard<'a> {
        cfg: &'a LfsConfig,
    }

    impl<'a> Drop for Guard<'a> {
        fn drop(&mut self) {
            if let Some(unlock_fn) = self.cfg.unlock {
                unsafe { unlock_fn(self.cfg as *const LfsConfig) };
            }
        }
    }

    let _guard = Guard { cfg };

    // Trace call (implementation omitted)
    // LFS_TRACE!("lfs_file_truncate({:p}, {:p}, {})", lfs, file, size);

    // Validate file is in open list
    debug_assert!(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

    // Perform actual truncation
    let err = lfs_file_truncate_(lfs, file, size);

    // Trace result (implementation omitted)
    // LFS_TRACE!("lfs_file_truncate -> {}", err);

    // Return error code (guard will handle unlock)
    err
}

//  lfs_file_tell
pub unsafe extern "C" fn lfs_file_tell(lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    let cfg = (*lfs).cfg;
    let lock = (*cfg).lock.unwrap();
    let err = lock(cfg);
    if err != 0 {
        return err as LfsSoff;
    }

    // LFS_TRACE macro would go here if implemented
    // LFS_ASSERT converted to Rust's assert!
    assert!(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist as *mut c_void));

    let res = lfs_file_tell_(lfs, file);

    // LFS_TRACE output would go here
    let unlock = (*cfg).unlock.unwrap();
    unlock(cfg);

    res
}

//  lfs_file_rewind
pub unsafe extern "C" fn lfs_file_rewind(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let cfg = (*lfs).cfg;
    
    // LOCK操作
    let lock_err = match ((*cfg).lock) {
        Some(lock_fn) => lock_fn(cfg),
        None => return LfsError::Inval as i32,
    };
    if lock_err != 0 {
        return lock_err;
    }

    // TRACE日志
    LFS_TRACE!("lfs_file_rewind({:p}, {:p})", lfs, file);

    // 调用内部实现
    let err = lfs_file_rewind_(lfs, file);

    // 结果日志
    LFS_TRACE!("lfs_file_rewind -> {}", err);

    // UNLOCK操作
    match ((*cfg).unlock) {
        Some(unlock_fn) => unlock_fn(cfg),
        None => return LfsError::Inval as i32,
    };

    err
}

//  lfs_file_size
pub unsafe extern "C" fn lfs_file_size(lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    let cfg = &(*lfs).cfg;
    
    // LOCK
    if let Some(lock_fn) = cfg.lock {
        let err = lock_fn(cfg);
        if err != 0 {
            return err as LfsSoff;
        }
    }
    
    // LFS_TRACE("lfs_file_size(%p, %p)", (void*)lfs, (void*)file);
    
    // LFS_ASSERT
    assert!(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));
    
    let res = lfs_file_size_(lfs, file);
    
    // LFS_TRACE("lfs_file_size -> %"PRId32, res);
    
    // UNLOCK
    if let Some(unlock_fn) = cfg.unlock {
        unlock_fn(cfg);
    }
    
    res
}

//  lfs_mkdir
pub unsafe extern "C" fn lfs_mkdir(lfs: *mut Lfs, path: *const c_char) -> i32 {
    // 获取配置引用
    let cfg = &(*lfs).cfg;
    let cfg_ref = &*cfg;

    // 尝试获取锁
    let err = match cfg_ref.lock {
        Some(lock) => lock(cfg as *const LfsConfig),
        None => 0,
    };
    if err != 0 {
        return err;
    }

    // 使用守卫确保解锁
    struct Guard<'a> {
        cfg: &'a LfsConfig,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            if let Some(unlock) = self.cfg.unlock {
                unsafe { unlock(self.cfg as *const LfsConfig) };
            }
        }
    }
    let _guard = Guard { cfg: cfg_ref };

    // 调用底层实现
    #[cfg(feature = "trace")]
    lfs_util::trace!("lfs_mkdir({:p}, {:?})", lfs, path);
    
    let err = lfs_mkdir_(lfs, path);
    
    #[cfg(feature = "trace")]
    lfs_util::trace!("lfs_mkdir -> {}", err);

    err
}

//  lfs_dir_open
pub unsafe extern "C" fn lfs_dir_open(lfs: *mut Lfs, dir: *mut LfsDir, path: *const c_char) -> i32 {
    // LOCK操作
    let cfg = &*(*lfs).cfg;
    let lock_err = match cfg.lock {
        Some(lock_fn) => lock_fn(cfg),
        None => 0,
    };
    if lock_err != 0 {
        return lock_err;
    }

    // 调试跟踪
    let path_str = std::ffi::CStr::from_ptr(path).to_string_lossy();
    lfs_util::trace!("lfs_dir_open({:p}, {:p}, \"{}\")", lfs, dir, path_str);

    // 断言目录未打开
    debug_assert!(!lfs_mlist_isopen((*lfs).mlist, dir as *mut LfsMlist));

    // 调用核心目录打开逻辑
    let err = lfs_dir_open_(lfs, dir, path);

    // 调试跟踪结果
    lfs_util::trace!("lfs_dir_open -> {}", err);

    // UNLOCK操作
    if let Some(unlock_fn) = cfg.unlock {
        unlock_fn(cfg);
    }

    err
}

//  lfs_dir_close
#[no_mangle]
pub unsafe extern "C" fn lfs_dir_close(lfs: *mut Lfs, dir: *mut LfsDir) -> i32 {
    let cfg = (*lfs).cfg;
    
    // Lock handling
    let err = match (*cfg).lock {
        Some(lock_fn) => lock_fn(cfg),
        None => 0,
    };
    if err != 0 {
        return err;
    }

    // Tracing
    lfs_util::lfs_trace!(
        "lfs_dir_close({:p}, {:p})",
        lfs as *mut c_void,
        dir as *mut c_void
    );

    // Actual close operation
    let err = lfs_dir_close_(lfs, dir);

    // Tracing result
    lfs_util::lfs_trace!("lfs_dir_close -> {}", err);

    // Unlock handling
    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }

    err
}

//  lfs_dir_read
pub unsafe extern "C" fn lfs_dir_read(lfs: *mut Lfs, dir: *mut LfsDir, info: *mut LfsInfo) -> i32 {
    let cfg = (*lfs).cfg;
    let err = if let Some(lock) = (*cfg).lock {
        lock(cfg)
    } else {
        0
    };
    if err != 0 {
        return err;
    }

    #[cfg(feature = "trace")]
    lfs_util::trace!("lfs_dir_read({:p}, {:p}, {:p})", lfs, dir, info);

    let err = lfs_dir_read_(lfs, dir, info);

    #[cfg(feature = "trace")]
    lfs_util::trace!("lfs_dir_read -> {}", err);

    if let Some(unlock) = (*cfg).unlock {
        unlock(cfg);
    }

    err
}

//  lfs_dir_seek
pub unsafe extern "C" fn lfs_dir_seek(lfs: *mut Lfs, dir: *mut LfsDir, off: LfsOff) -> i32 {
    let cfg = (*lfs).cfg;
    let cfg_ref = &*cfg;

    // Acquire lock
    let lock_fn = match cfg_ref.lock {
        Some(f) => f,
        None => return LfsError::Inval as i32,
    };
    let mut err = lock_fn(cfg);
    if err != 0 {
        return err;
    }

    // Trace call (implementation depends on logging setup)
    // lfs_util::trace!("lfs_dir_seek({:p}, {:p}, {})", lfs, dir, off);

    // Perform actual seek operation
    err = lfs_dir_seek_(lfs, dir, off);

    // Trace result (implementation depends on logging setup)
    // lfs_util::trace!("lfs_dir_seek -> {}", err);

    // Release lock
    if let Some(unlock_fn) = cfg_ref.unlock {
        unlock_fn(cfg);
    }

    err
}

//  lfs_dir_tell
pub unsafe extern "C" fn lfs_dir_tell(lfs: *mut Lfs, dir: *mut LfsDir) -> LfsSoff {
    let cfg = &(*lfs).cfg;
    
    // Acquire lock
    let err = if let Some(lock_fn) = cfg.lock {
        lock_fn(cfg)
    } else {
        0
    };
    if err != 0 {
        return err as LfsSoff;
    }

    // Trace entry
    lfs_util::trace!("lfs_dir_tell({:p}, {:p})", lfs as *mut c_void, dir as *mut c_void);

    // Call underlying implementation
    let res = lfs_dir_tell_(lfs, dir);

    // Trace result
    lfs_util::trace!("lfs_dir_tell -> {}", res);

    // Release lock
    if let Some(unlock_fn) = cfg.unlock {
        unlock_fn(cfg);
    }

    res
}

//  lfs_dir_rewind
pub unsafe extern "C" fn lfs_dir_rewind(lfs: *mut Lfs, dir: *mut LfsDir) -> LfsError {
    let cfg = (*lfs).cfg;
    
    // Acquire lock
    let lock_err = if let Some(lock_fn) = (*cfg).lock {
        lock_fn(cfg)
    } else {
        return LfsError::Inval;
    };
    
    if lock_err != 0 {
        return LfsError::try_from(lock_err).unwrap_or(LfsError::Io);
    }

    // LFS_TRACE would typically be handled with logging here
    // In practice this might be conditionally compiled debug output
    // lfs_util::trace!("lfs_dir_rewind({:p}, {:p})", lfs, dir);

    let err = lfs_dir_rewind_(lfs, dir);

    // LFS_TRACE for return value
    // lfs_util::trace!("lfs_dir_rewind -> {}", err as i32);

    // Release lock
    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }

    err
}

//  lfs_fs_stat
pub fn lfs_fs_stat(lfs: &mut Lfs, fsinfo: &mut LfsFsinfo) -> i32 {
    let cfg = unsafe { &*lfs.cfg };
    
    // LFS_LOCK
    let err = if let Some(lock_fn) = cfg.lock {
        unsafe { lock_fn(cfg) }
    } else {
        0
    };
    if err != 0 {
        return err;
    }

    // LFS_TRACE("lfs_fs_stat(%p, %p)", (void*)lfs, (void*)fsinfo);
    let err = lfs_fs_stat_(lfs, fsinfo);
    // LFS_TRACE("lfs_fs_stat -> %d", err);

    // LFS_UNLOCK
    if let Some(unlock_fn) = cfg.unlock {
        unsafe { unlock_fn(cfg) };
    }

    err
}

//  lfs_fs_size
pub unsafe extern "C" fn lfs_fs_size(lfs: *mut Lfs) -> LfsSsize {
    let cfg = (*lfs).cfg;
    
    // Lock the filesystem
    if let Some(lock_fn) = (*cfg).lock {
        let err = lock_fn(cfg);
        if err != 0 {
            return err as LfsSsize;
        }
    }

    // Log trace before operation
    lfs_util::trace!("lfs_fs_size({:p})", lfs);
    
    // Perform actual size calculation
    let res = lfs_fs_size_(lfs);
    
    // Log trace with result
    lfs_util::trace!("lfs_fs_size -> {}", res);
    
    // Unlock the filesystem
    if let Some(unlock_fn) = (*cfg).unlock {
        let _ = unlock_fn(cfg);
    }

    res
}

//  lfs_fs_traverse
pub unsafe extern "C" fn lfs_fs_traverse(
    lfs: *mut Lfs,
    cb: Option<unsafe extern "C" fn(*mut c_void, LfsBlock) -> i32>,
    data: *mut c_void,
) -> i32 {
    let lfs = &mut *lfs;
    let cfg = &*lfs.cfg;

    // LOCK
    let err = match cfg.lock {
        Some(lock_fn) => lock_fn(cfg as *const LfsConfig),
        None => 0,
    };
    if err != 0 {
        return err;
    }

    // TRACE
    lfs_util::trace(
        "lfs_fs_traverse(%p, %p, %p)",
        lfs as *const _ as *mut c_void,
        cb.map(|f| f as *const c_void).unwrap_or(core::ptr::null()) as *mut c_void,
        data,
    );

    // Call underlying implementation
    let err = lfs_fs_traverse_(lfs, cb, data, true);

    // TRACE result
    lfs_util::trace("lfs_fs_traverse -> %d", err);

    // UNLOCK
    if let Some(unlock_fn) = cfg.unlock {
        unlock_fn(cfg as *const LfsConfig);
    }

    err
}

//  lfs_fs_mkconsistent
pub unsafe extern "C" fn lfs_fs_mkconsistent(lfs: &mut Lfs) -> i32 {
    let cfg = lfs.cfg;
    let lock_err = if let Some(lock_fn) = (*cfg).lock { lock_fn(cfg) } else { 0 };
    if lock_err != 0 {
        return lock_err;
    }

    lfs_util::trace!("lfs_fs_mkconsistent({:p})", lfs as *mut _);
    
    let err = lfs_fs_mkconsistent_(lfs);
    
    lfs_util::trace!("lfs_fs_mkconsistent -> {}", err);
    
    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }
    
    err
}

//  lfs_fs_gc
pub unsafe extern "C" fn lfs_fs_gc(lfs: *mut Lfs) -> i32 {
    let cfg = (*lfs).cfg;
    let lock = (*cfg).lock;
    let err = if let Some(lock_fn) = lock {
        lock_fn(cfg)
    } else {
        0
    };
    if err != 0 {
        return err;
    }

    lfs_util::trace!("lfs_fs_gc({:p})", lfs);

    let err = lfs_fs_gc_(lfs);

    lfs_util::trace!("lfs_fs_gc -> {}", err);

    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }

    err
}

//  lfs_fs_grow
pub unsafe extern "C" fn lfs_fs_grow(lfs: *mut Lfs, block_count: LfsSize) -> LfsError {
    let cfg = (*lfs).cfg;
    
    // Acquire lock
    let lock = (*cfg).lock.ok_or(LfsError::Inval)?;
    let err = lock(cfg);
    if err != 0 {
        return std::mem::transmute(err);
    }

    // Trace call
    lfs_util::trace!("lfs_fs_grow({:p}, {})", lfs, block_count);

    // Perform actual grow operation
    let result = lfs_fs_grow_(lfs, block_count);

    // Trace result
    lfs_util::trace!("lfs_fs_grow -> {}", result as i32);

    // Release lock
    let unlock = (*cfg).unlock.ok_or(LfsError::Inval)?;
    unlock(cfg);

    result
}

//  lfs_migrate
pub unsafe extern "C" fn lfs_migrate(lfs: *mut Lfs, cfg: *const LfsConfig) -> i32 {
    // 调用锁函数
    let err = if let Some(lock_fn) = (*cfg).lock {
        lock_fn(cfg)
    } else {
        0
    };
    if err != 0 {
        return err;
    }

    // 跟踪日志输出配置信息
    LFS_TRACE!(
        "lfs_migrate({:p}, {:p} {{.context={:p}, .read={:p}, .prog={:p}, .erase={:p}, .sync={:p}, \
        .read_size={}, .prog_size={}, .block_size={}, .block_count={}, .block_cycles={}, \
        .cache_size={}, .lookahead_size={}, .read_buffer={:p}, .prog_buffer={:p}, \
        .lookahead_buffer={:p}, .name_max={}, .file_max={}, .attr_max={}}})",
        lfs as *const c_void,
        cfg as *const c_void,
        (*cfg).context,
        (*cfg).read.map(|f| f as usize as *const c_void).unwrap_or(core::ptr::null()),
        (*cfg).prog.map(|f| f as usize as *const c_void).unwrap_or(core::ptr::null()),
        (*cfg).erase.map(|f| f as usize as *const c_void).unwrap_or(core::ptr::null()),
        (*cfg).sync.map(|f| f as usize as *const c_void).unwrap_or(core::ptr::null()),
        (*cfg).read_size,
        (*cfg).prog_size,
        (*cfg).block_size,
        (*cfg).block_count,
        (*cfg).block_cycles,
        (*cfg).cache_size,
        (*cfg).lookahead_size,
        (*cfg).read_buffer,
        (*cfg).prog_buffer,
        (*cfg).lookahead_buffer,
        (*cfg).name_max,
        (*cfg).file_max,
        (*cfg).attr_max
    );

    // 执行实际的迁移操作
    let err = lfs_migrate_(lfs, cfg);

    // 跟踪日志输出结果
    LFS_TRACE!("lfs_migrate -> {}", err);

    // 调用解锁函数
    if let Some(unlock_fn) = (*cfg).unlock {
        unlock_fn(cfg);
    }

    err
}