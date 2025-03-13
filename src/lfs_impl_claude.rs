use crate::lfs_utils;
use core::ffi::c_void;
use crate::lfs_strucs;

/// Drop a cache. Does not zero the buffer as it may be reused with identical data.
#[inline]
pub fn lfs_cache_drop(lfs: &mut Lfs, rcache: &mut LfsCache) {
    // do not zero, cheaper if cache is readonly or only going to be
    // written with identical data (during relocates)
    let _ = lfs; // unused parameter
    rcache.block = LFS_BLOCK_NULL;
}

/// 将缓存内容清零以避免信息泄漏
pub fn lfs_cache_zero(lfs: &Lfs, pcache: &mut LfsCache) {
    // 使用0xff填充缓存缓冲区以避免信息泄漏
    unsafe {
        core::ptr::write_bytes(
            pcache.buffer,
            0xff,
            (*lfs.cfg).cache_size as usize
        );
    }
    pcache.block = LFS_BLOCK_NULL;
}

fn lfs_bd_read(
    lfs: &mut Lfs,
    pcache: Option<&LfsCache>,
    rcache: &mut LfsCache,
    hint: LfsSize,
    block: LfsBlock,
    off: LfsOff,
    buffer: *mut c_void,
    size: LfsSize,
) -> i32 {
    let mut data = buffer as *mut u8;
    let cfg = unsafe { &*lfs.cfg };
    
    if off + size > cfg.block_size || (lfs.block_count != 0 && block >= lfs.block_count) {
        return LfsError::Corrupt as i32;
    }

    let mut remaining_size = size;
    let mut current_off = off;
    
    while remaining_size > 0 {
        let mut diff = remaining_size;

        // 检查是否在pcache中
        if let Some(pc) = pcache {
            if block == pc.block && current_off < pc.off + pc.size {
                if current_off >= pc.off {
                    // 已经在pcache中
                    diff = lfs_util::min(diff, pc.size - (current_off - pc.off));
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            pc.buffer.add((current_off - pc.off) as usize),
                            data,
                            diff as usize,
                        );
                    }

                    unsafe { data = data.add(diff as usize) };
                    current_off += diff;
                    remaining_size -= diff;
                    continue;
                }

                // pcache优先
                diff = lfs_util::min(diff, pc.off - current_off);
            }
        }

        // 检查是否在rcache中
        if block == rcache.block && current_off < rcache.off + rcache.size {
            if current_off >= rcache.off {
                // 已经在rcache中
                diff = lfs_util::min(diff, rcache.size - (current_off - rcache.off));
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        rcache.buffer.add((current_off - rcache.off) as usize),
                        data,
                        diff as usize,
                    );
                }

                unsafe { data = data.add(diff as usize) };
                current_off += diff;
                remaining_size -= diff;
                continue;
            }

            // rcache优先
            diff = lfs_util::min(diff, rcache.off - current_off);
        }

        // 尝试直接读取，绕过缓存
        if remaining_size >= hint && current_off % cfg.read_size == 0 && remaining_size >= cfg.read_size {
            diff = lfs_util::aligndown(diff, cfg.read_size);
            let err = unsafe {
                (cfg.read.unwrap())(
                    lfs.cfg,
                    block,
                    current_off,
                    data as *mut c_void,
                    diff,
                )
            };
            if err != 0 {
                return err;
            }

            unsafe { data = data.add(diff as usize) };
            current_off += diff;
            remaining_size -= diff;
            continue;
        }

        // 加载到缓存
        debug_assert!(lfs.block_count == 0 || block < lfs.block_count);
        rcache.block = block;
        rcache.off = lfs_util::aligndown(current_off, cfg.read_size);
        rcache.size = lfs_util::min(
            lfs_util::min(
                lfs_util::alignup(current_off + hint, cfg.read_size),
                cfg.block_size,
            ) - rcache.off,
            cfg.cache_size,
        );
        
        let err = unsafe {
            (cfg.read.unwrap())(
                lfs.cfg,
                rcache.block,
                rcache.off,
                rcache.buffer as *mut c_void,
                rcache.size,
            )
        };
        
        debug_assert!(err <= 0);
        if err != 0 {
            return err;
        }
    }

    0
}

fn lfs_bd_cmp(
    lfs: &mut Lfs,
    pcache: &LfsCache,
    rcache: &mut LfsCache,
    hint: LfsSize,
    block: LfsBlock,
    off: LfsOff,
    buffer: *const c_void,
    size: LfsSize,
) -> i32 {
    let data = buffer as *const u8;
    let mut diff: LfsSize = 0;

    let mut i: LfsOff = 0;
    while i < size {
        let mut dat = [0u8; 8];

        diff = lfs_util::lfs_min(size - i as LfsSize, dat.len() as LfsSize);
        let err = lfs_bd_read(
            lfs,
            pcache,
            rcache,
            hint - i as LfsSize,
            block,
            off + i,
            dat.as_mut_ptr() as *mut c_void,
            diff,
        );
        if err != 0 {
            return err;
        }

        unsafe {
            let res = libc::memcmp(
                dat.as_ptr() as *const c_void,
                data.add(i as usize) as *const c_void,
                diff as usize,
            );
            if res != 0 {
                return if res < 0 {
                    LfsCmp::Lt as i32
                } else {
                    LfsCmp::Gt as i32
                };
            }
        }

        i += diff as LfsOff;
    }

    LfsCmp::Eq as i32
}

fn lfs_bd_crc(
    lfs: &mut Lfs,
    pcache: &LfsCache,
    rcache: &mut LfsCache,
    hint: LfsSize,
    block: LfsBlock,
    off: LfsOff,
    size: LfsSize,
    crc: &mut u32
) -> i32 {
    let mut diff = 0;

    for i in 0..size {
        if i >= diff {
            let mut dat = [0u8; 8];
            diff = core::cmp::min(size - i, dat.len() as LfsSize);
            let err = lfs_bd_read(
                lfs,
                pcache,
                rcache,
                hint - i,
                block,
                off + i,
                &mut dat[..diff as usize],
                diff
            );
            if err != 0 {
                return err;
            }

            *crc = lfs_util::lfs_crc(*crc, &dat[..diff as usize]);
        }
    }

    0
}

/// 将编程缓存刷新到磁盘，并可选择验证写入数据
fn lfs_bd_flush(lfs: &mut Lfs, pcache: &mut LfsCache, rcache: &mut LfsCache, validate: bool) -> i32 {
    if pcache.block != LFS_BLOCK_NULL && pcache.block != LFS_BLOCK_INLINE {
        debug_assert!(pcache.block < lfs.block_count);
        let diff = lfs_util::lfs_alignup(pcache.size, unsafe { (*lfs.cfg).prog_size });
        
        // 使用配置的编程函数将缓存写入磁盘
        let err = unsafe {
            match (*lfs.cfg).prog {
                Some(prog) => prog(
                    lfs.cfg,
                    pcache.block,
                    pcache.off,
                    pcache.buffer as *const c_void,
                    diff
                ),
                None => 0,
            }
        };
        
        debug_assert!(err <= 0);
        if err != 0 {
            return err;
        }

        if validate {
            // 检查磁盘上的数据
            lfs_cache_drop(lfs, rcache);
            let res = lfs_bd_cmp(
                lfs,
                core::ptr::null_mut(),
                rcache,
                diff,
                pcache.block,
                pcache.off,
                pcache.buffer as *const c_void,
                diff
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

/// 同步块设备并刷新缓存
/// 
/// 这个函数首先丢弃读缓存，然后刷新程序缓存，最后同步底层块设备
/// 
/// # 参数
/// * `lfs` - littlefs文件系统实例
/// * `pcache` - 程序缓存
/// * `rcache` - 读取缓存
/// * `validate` - 是否验证缓存数据
/// 
/// # 返回
/// * 成功时返回0，错误时返回负错误码
fn lfs_bd_sync(
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

    // 调用底层的同步操作
    let err = unsafe { ((*lfs.cfg).sync.unwrap())(lfs.cfg) };
    debug_assert!(err <= 0);
    err
}

fn lfs_bd_prog(lfs: &mut Lfs, 
               pcache: &mut LfsCache, 
               rcache: &mut LfsCache, 
               validate: bool,
               block: LfsBlock, 
               off: LfsOff,
               buffer: *const c_void, 
               size: LfsSize) -> i32 {
    let data = buffer as *const u8;
    debug_assert!(block == LFS_BLOCK_INLINE || block < lfs.block_count);
    debug_assert!(off + size <= unsafe { (*lfs.cfg).block_size });

    let mut current_data = data;
    let mut current_off = off;
    let mut remaining_size = size;

    while remaining_size > 0 {
        if block == pcache.block &&
           current_off >= pcache.off &&
           current_off < pcache.off + unsafe { (*lfs.cfg).cache_size } {
            // already fits in pcache?
            let diff = core::cmp::min(
                remaining_size,
                unsafe { (*lfs.cfg).cache_size } - (current_off - pcache.off)
            );
            
            unsafe {
                core::ptr::copy_nonoverlapping(
                    current_data,
                    pcache.buffer.add((current_off - pcache.off) as usize),
                    diff as usize
                );
            }

            unsafe { current_data = current_data.add(diff as usize); }
            current_off += diff;
            remaining_size -= diff;

            pcache.size = core::cmp::max(pcache.size, current_off - pcache.off);
            if pcache.size == unsafe { (*lfs.cfg).cache_size } {
                // eagerly flush out pcache if we fill up
                let err = lfs_bd_flush(lfs, pcache, rcache, validate);
                if err != 0 {
                    return err;
                }
            }

            continue;
        }

        // pcache must have been flushed, either by programming and
        // entire block or manually flushing the pcache
        debug_assert!(pcache.block == LFS_BLOCK_NULL);

        // prepare pcache, first condition can no longer fail
        pcache.block = block;
        pcache.off = lfs_util::lfs_aligndown(current_off, unsafe { (*lfs.cfg).prog_size });
        pcache.size = 0;
    }

    0
}

/// 擦除指定块
/// 
/// 参数:
/// - lfs: 文件系统实例
/// - block: 块编号
/// 
/// 返回:
/// - Ok(()) 成功
/// - Err(LfsError) 错误码
fn lfs_bd_erase(lfs: &Lfs, block: LfsBlock) -> Result<(), LfsError> {
    debug_assert!(block < lfs.block_count);
    
    let err = unsafe {
        if let Some(erase) = (*lfs.cfg).erase {
            erase(lfs.cfg, block)
        } else {
            return Err(LfsError::Io);
        }
    };
    
    debug_assert!(err <= 0);
    
    if err < 0 {
        // 将C错误码转换为Rust Result
        match err {
            -5 => Err(LfsError::Io),
            -84 => Err(LfsError::Corrupt),
            _ => Err(LfsError::Io), // 默认映射到IO错误
        }
    } else {
        Ok(())
    }
}

/// 获取路径中第一个名称的长度（直到遇到 "/" 为止）
pub fn lfs_path_namelen(path: &str) -> LfsSize {
    path.find('/').unwrap_or(path.len()) as LfsSize
}

/// 检查路径是否为最后一个节点
#[inline]
pub fn lfs_path_islast(path: &[u8]) -> bool {
    let namelen = lfs_path_namelen(path);
    let remaining = &path[namelen..];
    
    // 找到第一个非'/'字符的位置
    let slash_span = remaining.iter().take_while(|&&c| c == b'/').count();
    
    // 检查剩余部分是否全部是结束符
    remaining.get(slash_span).copied() == Some(0) || remaining.get(slash_span).is_none()
}

/// 检查给定路径是否表示一个目录
/// 
/// 如果路径是目录则返回true，否则返回false
#[inline]
pub fn lfs_path_isdir(path: &[u8]) -> bool {
    let name_len = lfs_path_namelen(path);
    path.len() > name_len && path[name_len] != 0
}

/// 交换块对中的两个元素
#[inline]
fn lfs_pair_swap(pair: &mut [LfsBlock; 2]) {
    let t = pair[0];
    pair[0] = pair[1];
    pair[1] = t;
}

/// 检查一个区块对是否包含空区块
#[inline]
pub fn lfs_pair_isnull(pair: &[LfsBlock; 2]) -> bool {
    pair[0] == LFS_BLOCK_NULL || pair[1] == LFS_BLOCK_NULL
}

/// 比较两个块对是否相同
///
/// 当两个块对不共享任何块时返回 true（不相同）
/// 当两个块对共享至少一个块时返回 false（相同）
#[inline]
pub fn lfs_pair_cmp(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> bool {
    !(paira[0] == pairb[0] || paira[1] == pairb[1] ||
      paira[0] == pairb[1] || paira[1] == pairb[0])
}

/// 检查两对块对是否同步（无论顺序如何）
///
/// 当两对块包含相同的块（无论顺序）时返回true
#[inline]
pub fn lfs_pair_issync(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> bool {
    (paira[0] == pairb[0] && paira[1] == pairb[1]) ||
    (paira[0] == pairb[1] && paira[1] == pairb[0])
}

/// 将内存中的一对块号从小端字节序转换为主机字节序
pub fn lfs_pair_fromle32(pair: &mut [LfsBlock; 2]) {
    pair[0] = lfs_util::fromle32(pair[0]);
    pair[1] = lfs_util::fromle32(pair[1]);
}

#[inline]
pub fn lfs_pair_tole32(pair: &mut [LfsBlock; 2]) {
    pair[0] = lfs_util::lfs_tole32(pair[0]);
    pair[1] = lfs_util::lfs_tole32(pair[1]);
}

/// 检查tag是否有效
#[inline]
pub fn lfs_tag_isvalid(tag: LfsTag) -> bool {
    !(tag & 0x80000000) != 0
}

/// 检查一个标签是否为删除标签
#[inline]
pub fn lfs_tag_isdelete(tag: LfsTag) -> bool {
    ((tag << 22) as i32) >> 22 == -1
}

#[inline]
pub fn lfs_tag_type1(tag: LfsTag) -> u16 {
    ((tag & 0x70000000) >> 20) as u16
}

#[inline]
fn lfs_tag_type2(tag: LfsTag) -> u16 {
    ((tag & 0x78000000) >> 20) as u16
}

/// 从标签中提取type3字段（位20-30）
#[inline]
pub fn lfs_tag_type3(tag: LfsTag) -> u16 {
    ((tag & 0x7ff00000) >> 20) as u16
}

/// 从tag中提取chunk信息
pub fn lfs_tag_chunk(tag: LfsTag) -> u8 {
    ((tag & 0x0ff00000) >> 20) as u8
}

#[inline]
pub fn lfs_tag_splice(tag: LfsTag) -> i8 {
    lfs_tag_chunk(tag) as i8
}

/// 从标签中提取ID
#[inline]
pub fn lfs_tag_id(tag: LfsTag) -> u16 {
    ((tag & 0x000ffc00) >> 10) as u16
}

#[inline]
pub fn lfs_tag_size(tag: LfsTag) -> LfsSize {
    tag & 0x000003ff
}

#[inline]
fn lfs_tag_dsize(tag: LfsTag) -> LfsSize {
    core::mem::size_of::<LfsTag>() as LfsSize + lfs_tag_size(tag + lfs_tag_isdelete(tag))
}

/// XOR operation on gstate, equivalent to a bitwise XOR of the underlying 32-bit values
pub fn lfs_gstate_xor(a: &mut LfsGstate, b: &LfsGstate) {
    // Reinterpret as array of u32 and perform XOR operations
    unsafe {
        let a_ptr = a as *mut LfsGstate as *mut u32;
        let b_ptr = b as *const LfsGstate as *const u32;
        
        for i in 0..3 {
            *a_ptr.add(i) ^= *b_ptr.add(i);
        }
    }
}

/// 检查全局状态是否为零
#[inline]
pub fn lfs_gstate_iszero(a: &LfsGstate) -> bool {
    // 将LfsGstate视为u32数组检查所有三个字段是否为0
    let ptr = a as *const LfsGstate as *const u32;
    for i in 0..3 {
        unsafe {
            if *ptr.add(i) != 0 {
                return false;
            }
        }
    }
    true
}

/// 检查全局状态是否有孤立节点
#[inline]
pub fn lfs_gstate_hasorphans(a: &LfsGstate) -> bool {
    lfs_tag_size(a.tag) != 0
}

#[inline]
pub fn lfs_gstate_getorphans(a: &LfsGstate) -> u8 {
    (lfs_tag_size(a.tag) & 0x1ff) as u8
}

/// 检查全局状态是否有移动标志
#[inline]
pub fn lfs_gstate_hasmove(a: &LfsGstate) -> bool {
    lfs_tag_type1(a.tag)
}

#[inline]
pub fn lfs_gstate_needssuperblock(a: &LfsGstate) -> bool {
    lfs_tag_size(a.tag) >> 9 != 0
}

#[inline]
pub fn lfs_gstate_hasmovehere(a: &LfsGstate, pair: &[LfsBlock; 2]) -> bool {
    lfs_tag_type1(a.tag) && lfs_pair_cmp(&a.pair, pair) == 0
}

/// 将little-endian格式的全局状态转换为主机字节序
#[inline]
pub fn lfs_gstate_fromle32(a: &mut LfsGstate) {
    a.tag = lfs_util::lfs_fromle32(a.tag);
    a.pair[0] = lfs_util::lfs_fromle32(a.pair[0]);
    a.pair[1] = lfs_util::lfs_fromle32(a.pair[1]);
}

/// 将LfsGstate结构转换为小端字节序
#[inline]
fn lfs_gstate_tole32(a: &mut LfsGstate) {
    a.tag = lfs_util::lfs_tole32(a.tag);
    a.pair[0] = lfs_util::lfs_tole32(a.pair[0]);
    a.pair[1] = lfs_util::lfs_tole32(a.pair[1]);
}

/// 将 LfsFcrc 结构从小端序转换为本地字节序
fn lfs_fcrc_fromle32(fcrc: &mut LfsFcrc) {
    fcrc.size = lfs_util::lfs_fromle32(fcrc.size);
    fcrc.crc = lfs_util::lfs_fromle32(fcrc.crc);
}

/// 将 LfsFcrc 结构转换为小端格式
fn lfs_fcrc_tole32(fcrc: &mut LfsFcrc) {
    fcrc.size = lfs_util::lfs_tole32(fcrc.size);
    fcrc.crc = lfs_util::lfs_tole32(fcrc.crc);
}

/// 将ctz结构中的字段从小端字节序转换为本地字节序
fn lfs_ctz_fromle32(ctz: &mut LfsCtz) {
    ctz.head = lfs_util::fromle32(ctz.head);
    ctz.size = lfs_util::fromle32(ctz.size);
}

/// 将CTZ结构转换为小端字节序
fn lfs_ctz_tole32(ctz: &mut LfsCtz) {
    ctz.head = lfs_util::lfs_tole32(ctz.head);
    ctz.size = lfs_util::lfs_tole32(ctz.size);
}

/// Converts a superblock structure from little-endian to native endian
#[inline]
pub fn lfs_superblock_fromle32(superblock: &mut LfsSuperblock) {
    superblock.version = lfs_util::lfs_fromle32(superblock.version);
    superblock.block_size = lfs_util::lfs_fromle32(superblock.block_size);
    superblock.block_count = lfs_util::lfs_fromle32(superblock.block_count);
    superblock.name_max = lfs_util::lfs_fromle32(superblock.name_max);
    superblock.file_max = lfs_util::lfs_fromle32(superblock.file_max);
    superblock.attr_max = lfs_util::lfs_fromle32(superblock.attr_max);
}

/// 将超级块结构中的字段从主机字节序转换为小端字节序
#[inline]
fn lfs_superblock_tole32(superblock: &mut LfsSuperblock) {
    superblock.version = lfs_util::lfs_tole32(superblock.version);
    superblock.block_size = lfs_util::lfs_tole32(superblock.block_size);
    superblock.block_count = lfs_util::lfs_tole32(superblock.block_count);
    superblock.name_max = lfs_util::lfs_tole32(superblock.name_max);
    superblock.file_max = lfs_util::lfs_tole32(superblock.file_max);
    superblock.attr_max = lfs_util::lfs_tole32(superblock.attr_max);
}

/// 检查节点是否在挂载列表中打开
///
/// 遍历挂载列表，检查指定节点是否存在
///
/// # Safety
/// 该函数假设指针参数有效且非空
fn lfs_mlist_isopen(head: *mut LfsMlist, node: *const LfsMlist) -> bool {
    let mut p = &mut head;
    
    unsafe {
        while !(*p).is_null() {
            if *p as *const LfsMlist == node {
                return true;
            }
            p = &mut (**p).next;
        }
    }
    
    false
}

/// 从 mlist 链表中移除指定的 mlist 节点
pub fn lfs_mlist_remove(lfs: &mut Lfs, mlist: *mut LfsMlist) {
    let mut p = &mut lfs.mlist;
    while !p.is_null() {
        if *p == mlist {
            unsafe {
                *p = (*(*p)).next;
            }
            break;
        }
        unsafe {
            p = &mut (*(*p)).next;
        }
    }
}

/// 将挂载列表项追加到文件系统的挂载列表
/// 
/// # Arguments
/// 
/// * `lfs` - 指向LittleFS文件系统的可变引用
/// * `mlist` - 指向要追加的挂载列表项的可变指针
pub unsafe fn lfs_mlist_append(lfs: *mut Lfs, mlist: *mut LfsMlist) {
    (*mlist).next = (*lfs).mlist;
    (*lfs).mlist = mlist;
}

/// 获取磁盘版本
fn lfs_fs_disk_version(lfs: &Lfs) -> u32 {
    #[cfg(feature = "MULTIVERSION")]
    if unsafe { (*lfs.cfg).disk_version != 0 } {
        return unsafe { (*lfs.cfg).disk_version };
    }
    
    LFS_DISK_VERSION
}

/// 获取文件系统的磁盘版本的主版本号
///
/// # Arguments
///
/// * `lfs` - 文件系统实例的引用
///
/// # Returns
///
/// 返回磁盘版本的主版本号
fn lfs_fs_disk_version_major(lfs: &Lfs) -> u16 {
    0xffff & (lfs_fs_disk_version(lfs) >> 16)
}

/// 获取磁盘版本的次要版本号
pub fn lfs_fs_disk_version_minor(lfs: &mut Lfs) -> u16 {
    (lfs_fs_disk_version(lfs) & 0xffff) as u16
}

/// 设置前瞻检查点为块计数
fn lfs_alloc_ckpoint(lfs: &mut Lfs) {
    lfs.lookahead.ckpoint = lfs.block_count;
}

/// 释放分配器的前瞻缓冲区资源
pub fn lfs_alloc_drop(lfs: &mut Lfs) {
    lfs.lookahead.size = 0;
    lfs.lookahead.next = 0;
    lfs_alloc_ckpoint(lfs);
}

fn lfs_alloc_lookahead(p: *mut c_void, block: LfsBlock) -> i32 {
    let lfs = unsafe { &mut *(p as *mut Lfs) };
    let off = ((block - lfs.lookahead.start) + lfs.block_count) % lfs.block_count;

    if off < lfs.lookahead.size {
        unsafe {
            *lfs.lookahead.buffer.add((off / 8) as usize) |= 1u8 << (off % 8);
        }
    }

    0
}

/// 扫描分配，重新初始化前瞻缓冲区并填充它
unsafe fn lfs_alloc_scan(lfs: *mut Lfs) -> i32 {
    // 将前瞻缓冲区移动到第一个未使用的块
    //
    // 注意我们将前瞻缓冲区限制为最多检查点的块数量
    // 这可防止lfs_alloc中的数学计算下溢
    (*lfs).lookahead.start = ((*lfs).lookahead.start + (*lfs).lookahead.next) 
            % (*lfs).block_count;
    (*lfs).lookahead.next = 0;
    (*lfs).lookahead.size = lfs_util::lfs_min(
            8 * (*(*lfs).cfg).lookahead_size,
            (*lfs).lookahead.ckpoint);

    // 从树中找到空闲块的掩码
    core::ptr::write_bytes((*lfs).lookahead.buffer, 0, (*(*lfs).cfg).lookahead_size as usize);
    let err = lfs_fs_traverse_(lfs, lfs_alloc_lookahead, lfs, true);
    if err != 0 {
        lfs_alloc_drop(lfs);
        return err;
    }

    0
}

/// 从系统中分配一个块
/// 
/// 扫描预读缓冲区寻找空闲块，必要时更新预读缓冲区
fn lfs_alloc(lfs: &mut Lfs, block: &mut LfsBlock) -> i32 {
    loop {
        // 扫描预读缓冲区寻找空闲块
        while lfs.lookahead.next < lfs.lookahead.size {
            let byte_index = (lfs.lookahead.next / 8) as usize;
            let bit_mask = 1u8 << (lfs.lookahead.next % 8);
            
            // 检查此块是否空闲
            if unsafe { *lfs.lookahead.buffer.add(byte_index) & bit_mask == 0 } {
                // 找到一个空闲块
                *block = (lfs.lookahead.start + lfs.lookahead.next) % lfs.block_count;

                // 贪婪查找下一个空闲块以最大化lfs_alloc_ckpoint能够提供用于扫描的块数
                loop {
                    lfs.lookahead.next += 1;
                    lfs.lookahead.ckpoint -= 1;

                    if lfs.lookahead.next >= lfs.lookahead.size {
                        return 0;
                    }
                    
                    let next_byte_index = (lfs.lookahead.next / 8) as usize;
                    let next_bit_mask = 1u8 << (lfs.lookahead.next % 8);
                    
                    if unsafe { *lfs.lookahead.buffer.add(next_byte_index) & next_bit_mask == 0 } {
                        return 0;
                    }
                }
            }

            lfs.lookahead.next += 1;
            lfs.lookahead.ckpoint -= 1;
        }

        // 为了防止我们的块分配器在文件系统满时无限循环，我们在开始一组分配之前
        // 用检查点标记没有进行中分配的点。
        //
        // 如果我们自上次检查点以来已经查看了所有块，我们报告存储空间不足。
        //
        if lfs.lookahead.ckpoint <= 0 {
            lfs_util::trace!("No more free space 0x{:x}", 
                   (lfs.lookahead.start + lfs.lookahead.next) % lfs.block_count);
            return LfsError::NoSpc as i32;
        }

        // 预读缓冲区中没有块，我们需要扫描文件系统以查找
        // 下一个预读窗口中未使用的块。
        let err = lfs_alloc_scan(lfs);
        if err != 0 {
            return err;
        }
    }
}

fn lfs_dir_getslice(lfs: &mut Lfs, dir: &LfsMdir, 
                    gmask: LfsTag, gtag: LfsTag,
                    goff: LfsOff, gbuffer: *mut c_void, gsize: LfsSize) -> LfsStag {
    let mut off = dir.off;
    let mut ntag = dir.etag;
    let mut gdiff: LfsStag = 0;

    // synthetic moves
    if lfs_gstate_hasmovehere(&lfs.gdisk, &dir.pair) &&
            lfs_tag_id(gmask) != 0 {
        if lfs_tag_id(lfs.gdisk.tag) == lfs_tag_id(gtag) {
            return LfsError::NoEnt as LfsStag;
        } else if lfs_tag_id(lfs.gdisk.tag) < lfs_tag_id(gtag) {
            gdiff -= LFS_MKTAG(0, 1, 0) as LfsStag;
        }
    }

    // iterate over dir block backwards (for faster lookups)
    while off >= core::mem::size_of::<LfsTag>() as LfsOff + lfs_tag_dsize(ntag) {
        off -= lfs_tag_dsize(ntag);
        let tag = ntag;
        let err = lfs_bd_read(lfs,
                null_mut(), &mut lfs.rcache, core::mem::size_of::<LfsTag>() as LfsSize,
                dir.pair[0], off, &mut ntag as *mut _ as *mut c_void, core::mem::size_of::<LfsTag>() as LfsSize);
        if err < 0 {
            return err;
        }

        ntag = (lfs_frombe32(ntag) ^ tag) & 0x7fffffff;

        if lfs_tag_id(gmask) != 0 &&
                lfs_tag_type1(tag) == LfsType::Splice as u32 &&
                lfs_tag_id(tag) <= lfs_tag_id((gtag as LfsStag - gdiff) as LfsTag) {
            if tag == (LFS_MKTAG(LfsType::Create as u32, 0, 0) |
                    (LFS_MKTAG(0, 0x3ff, 0) & (gtag as LfsStag - gdiff) as LfsTag)) {
                // found where we were created
                return LfsError::NoEnt as LfsStag;
            }

            // move around splices
            gdiff += LFS_MKTAG(0, lfs_tag_splice(tag), 0) as LfsStag;
        }

        if (gmask & tag) == (gmask & (gtag as LfsStag - gdiff) as LfsTag) {
            if lfs_tag_isdelete(tag) {
                return LfsError::NoEnt as LfsStag;
            }

            let diff = core::cmp::min(lfs_tag_size(tag), gsize);
            let err = lfs_bd_read(lfs,
                    null_mut(), &mut lfs.rcache, diff,
                    dir.pair[0], off + core::mem::size_of::<LfsTag>() as LfsOff + goff, 
                    gbuffer, diff);
            if err < 0 {
                return err;
            }

            unsafe {
                core::ptr::write_bytes(
                    (gbuffer as *mut u8).offset(diff as isize), 
                    0, 
                    (gsize - diff) as usize
                );
            }

            return tag as LfsStag + gdiff;
        }
    }

    LfsError::NoEnt as LfsStag
}

/// 获取目录中的元数据条目
/// 
/// 从指定目录中获取符合掩码和标签要求的元数据条目。
/// 
/// 这是对 lfs_dir_getslice 的简便包装，读取整个标签大小。
fn lfs_dir_get(lfs: &mut Lfs, dir: &LfsMdir, 
               gmask: LfsTag, gtag: LfsTag, 
               buffer: *mut c_void) -> LfsStag {
    lfs_dir_getslice(lfs, dir, 
                     gmask, gtag, 
                     0, buffer, lfs_tag_size(gtag))
}

fn lfs_dir_getread(
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
    if off + size > unsafe { (*lfs.cfg).block_size } {
        return LfsError::Corrupt as i32;
    }

    let mut remaining_size = size;
    let mut current_off = off;
    let mut current_data = data;

    while remaining_size > 0 {
        let mut diff = remaining_size;

        if let Some(pc) = pcache {
            if pc.block == LFS_BLOCK_INLINE && current_off < pc.off + pc.size {
                if current_off >= pc.off {
                    // is already in pcache?
                    diff = core::cmp::min(diff, pc.size - (current_off - pc.off));
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            pc.buffer.offset((current_off - pc.off) as isize),
                            current_data,
                            diff as usize,
                        );
                    }

                    current_data = unsafe { current_data.offset(diff as isize) };
                    current_off += diff;
                    remaining_size -= diff;
                    continue;
                }

                // pcache takes priority
                diff = core::cmp::min(diff, pc.off - current_off);
            }
        }

        if rcache.block == LFS_BLOCK_INLINE && current_off < rcache.off + rcache.size {
            if current_off >= rcache.off {
                // is already in rcache?
                diff = core::cmp::min(diff, rcache.size - (current_off - rcache.off));
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        rcache.buffer.offset((current_off - rcache.off) as isize),
                        current_data,
                        diff as usize,
                    );
                }

                current_data = unsafe { current_data.offset(diff as isize) };
                current_off += diff;
                remaining_size -= diff;
                continue;
            }

            // rcache takes priority
            diff = core::cmp::min(diff, rcache.off - current_off);
        }

        // load to cache, first condition can no longer fail
        rcache.block = LFS_BLOCK_INLINE;
        rcache.off = lfs_util::lfs_aligndown(current_off, unsafe { (*lfs.cfg).read_size });
        rcache.size = core::cmp::min(
            lfs_util::lfs_alignup(current_off + hint, unsafe { (*lfs.cfg).read_size }),
            unsafe { (*lfs.cfg).cache_size },
        );
        
        let err = lfs_dir_getslice(lfs, dir, gmask, gtag, rcache.off, rcache.buffer, rcache.size);
        if err < 0 {
            return err;
        }
    }

    0
}

/// 目录遍历过滤器函数，用于检查标签是否与过滤器匹配
/// 
/// # 参数
/// * `p` - 指向过滤标签的指针
/// * `tag` - 当前标签
/// * `buffer` - 数据缓冲区(未使用)
/// 
/// # 返回
/// * `true` - 如果标签应被过滤掉
/// * `false` - 否则
fn lfs_dir_traverse_filter(p: *mut c_void, tag: LfsTag, buffer: *const c_void) -> bool {
    let filtertag = unsafe { &mut *(p as *mut LfsTag) };
    let _ = buffer; // 忽略buffer参数

    // 根据标签结构中的unique位确定使用哪个掩码
    let mask = if (tag & LFS_MKTAG(0x100, 0, 0)) != 0 {
        LFS_MKTAG(0x7ff, 0x3ff, 0)
    } else {
        LFS_MKTAG(0x700, 0x3ff, 0)
    };

    // 检查是否有冗余
    if (mask & tag) == (mask & *filtertag) ||
       lfs_tag_isdelete(*filtertag) ||
       (LFS_MKTAG(0x7ff, 0x3ff, 0) & tag) == (
           LFS_MKTAG(LfsType::Delete as u16 as u32, 0, 0) |
           (LFS_MKTAG(0, 0x3ff, 0) & *filtertag)) {
        *filtertag = LFS_MKTAG(LfsType::FromNoop as u16 as u32, 0, 0);
        return true;
    }

    // 检查是否需要为创建/删除的标签进行调整
    if lfs_tag_type1(tag) == LfsType::Splice as u16 as u32 &&
       lfs_tag_id(tag) <= lfs_tag_id(*filtertag) {
        *filtertag += LFS_MKTAG(0, lfs_tag_splice(tag), 0);
    }

    false
}

/// 遍历目录结构，对符合条件的项目执行回调函数
fn lfs_dir_traverse(
    lfs: &mut Lfs,
    dir: &LfsMdir,
    off: LfsOff,
    ptag: LfsTag,
    attrs: *const LfsMattr,
    attrcount: i32,
    tmask: LfsTag,
    ttag: LfsTag,
    begin: u16,
    end: u16,
    diff: i16,
    cb: Option<unsafe extern "C" fn(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32>,
    data: *mut c_void,
) -> i32 {
    // 这个函数本质上是递归的，但有界。为了允许基于工具的分析而不增加不必要的代码成本，我们使用显式栈
    let mut stack = [LfsDirTraverse::default(); LFS_DIR_TRAVERSE_DEPTH - 1];
    let mut sp = 0;
    let mut res;

    // 迭代目录和属性
    let mut tag;
    let mut buffer;
    let mut disk = LfsDiskoff { block: 0, off: 0 };
    let mut p_off = off;
    let mut p_ptag = ptag;
    let mut p_attrs = attrs;
    let mut p_attrcount = attrcount;
    let mut p_tmask = tmask;
    let mut p_ttag = ttag;
    let mut p_begin = begin;
    let mut p_end = end;
    let mut p_diff = diff;
    let mut p_cb = cb;
    let mut p_data = data;

    loop {
        // 处理标签获取
        if p_off + lfs_tag_dsize(p_ptag) < dir.off {
            p_off += lfs_tag_dsize(p_ptag);
            let mut tag_val: LfsTag = 0;
            let err = lfs_bd_read(
                lfs,
                null_mut(),
                &lfs.rcache,
                core::mem::size_of::<LfsTag>() as u32,
                dir.pair[0],
                p_off,
                &mut tag_val as *mut LfsTag as *mut c_void,
                core::mem::size_of::<LfsTag>() as u32,
            );
            if err != 0 {
                return err;
            }

            tag = (lfs_frombe32(tag_val) ^ p_ptag) | 0x80000000;
            disk.block = dir.pair[0];
            disk.off = p_off + core::mem::size_of::<LfsTag>() as u32;
            buffer = &disk as *const _ as *const c_void;
            p_ptag = tag;
        } else if p_attrcount > 0 {
            let attrs_slice = unsafe { core::slice::from_raw_parts(p_attrs, p_attrcount as usize) };
            tag = attrs_slice[0].tag;
            buffer = attrs_slice[0].buffer;
            p_attrs = unsafe { p_attrs.add(1) };
            p_attrcount -= 1;
        } else {
            // 遍历完成，从栈中弹出？
            res = 0;
            break;
        }

        // 我们需要过滤吗？
        let mask: LfsTag = 0x7ff << 20;
        if (mask & p_tmask & tag) != (mask & p_tmask & p_ttag) {
            continue;
        }

        if lfs_tag_id(p_tmask) != 0 {
            debug_assert!(sp < LFS_DIR_TRAVERSE_DEPTH);
            // 递归，扫描重复项，并根据创建/删除更新标签
            stack[sp] = LfsDirTraverse {
                dir: dir as *const _,
                off: p_off,
                ptag: p_ptag,
                attrs: p_attrs,
                attrcount: p_attrcount,
                tmask: p_tmask,
                ttag: p_ttag,
                begin: p_begin,
                end: p_end,
                diff: p_diff,
                cb: p_cb,
                data: p_data,
                tag,
                buffer,
                disk,
            };
            sp += 1;

            p_tmask = 0;
            p_ttag = 0;
            p_begin = 0;
            p_end = 0;
            p_diff = 0;
            p_cb = Some(lfs_dir_traverse_filter);
            p_data = &mut stack[sp - 1].tag as *mut _ as *mut c_void;
            continue;
        }

        'popped: loop {
            // 在过滤范围内？
            if lfs_tag_id(p_tmask) != 0 && 
               !(lfs_tag_id(tag) >= p_begin && lfs_tag_id(tag) < p_end) {
                break 'popped;
            }

            // 处理MCU端操作的特殊情况
            match lfs_tag_type3(tag) {
                LfsType::FromNoop as u8 => {
                    // 不执行任何操作
                }
                LfsType::FromMove as u8 => {
                    // 如果不检查此条件，lfs_dir_traverse在重命名时可能会表现出极其昂贵的O(n^3)嵌套循环
                    // 这是因为lfs_dir_traverse尝试通过源目录中的标签过滤标签，触发带有自己的过滤操作的第二个lfs_dir_traverse
                    //
                    // 使用提交进行遍历
                    // '-> 使用过滤器进行遍历
                    //     '-> 使用移动进行遍历
                    //         '-> 使用过滤器进行遍历
                    //
                    // 然而，我们实际上并不关心过滤第二组标签，因为重复的标签在过滤时没有影响
                    //
                    // 这个检查显式跳过这种不必要的递归过滤，将运行时从O(n^3)减少到O(n^2)
                    if let Some(filter_cb) = p_cb {
                        if filter_cb as usize == lfs_dir_traverse_filter as usize {
                            break 'popped;
                        }
                    }

                    // 递归进入移动
                    stack[sp] = LfsDirTraverse {
                        dir: dir as *const _,
                        off: p_off,
                        ptag: p_ptag,
                        attrs: p_attrs,
                        attrcount: p_attrcount,
                        tmask: p_tmask,
                        ttag: p_ttag,
                        begin: p_begin,
                        end: p_end,
                        diff: p_diff,
                        cb: p_cb,
                        data: p_data,
                        tag: LFS_MKTAG(LfsType::FromNoop as u8, 0, 0),
                        buffer: null(),
                        disk: LfsDiskoff { block: 0, off: 0 },
                    };
                    sp += 1;

                    let fromid = lfs_tag_size(tag);
                    let toid = lfs_tag_id(tag);
                    let move_dir = unsafe { &*(buffer as *const LfsMdir) };
                    p_off = 0;
                    p_ptag = 0xffffffff;
                    p_attrs = null();
                    p_attrcount = 0;
                    p_tmask = LFS_MKTAG(0x600, 0x3ff, 0);
                    p_ttag = LFS_MKTAG(LfsType::Struct as u8, 0, 0);
                    p_begin = fromid;
                    p_end = fromid + 1;
                    p_diff = toid - fromid + p_diff;
                    dir = move_dir;
                }
                LfsType::FromUserAttrs as u8 => {
                    for i in 0..lfs_tag_size(tag) {
                        let attrs = unsafe { &*(buffer as *const LfsAttr).add(i as usize) };
                        if let Some(callback) = p_cb {
                            unsafe {
                                res = callback(
                                    p_data,
                                    LFS_MKTAG(LfsType::UserAttr as u8 + attrs.type_ as u32, 
                                             lfs_tag_id(tag) + p_diff as u32, 
                                             attrs.size),
                                    attrs.buffer as *const c_void,
                                );
                            }
                            if res < 0 {
                                return res;
                            }

                            if res != 0 {
                                break;
                            }
                        }
                    }
                }
                _ => {
                    if let Some(callback) = p_cb {
                        unsafe {
                            res = callback(
                                p_data,
                                tag + LFS_MKTAG(0, p_diff as u32, 0),
                                buffer,
                            );
                        }
                        if res < 0 {
                            return res;
                        }

                        if res != 0 {
                            break;
                        }
                    }
                }
            }
            break 'popped;
        }
    }

    if sp > 0 {
        // 从栈中弹出并返回，幸运的是所有弹出操作共享一个目标
        let popped = &stack[sp - 1];
        dir = unsafe { &*popped.dir };
        p_off = popped.off;
        p_ptag = popped.ptag;
        p_attrs = popped.attrs;
        p_attrcount = popped.attrcount;
        p_tmask = popped.tmask;
        p_ttag = popped.ttag;
        p_begin = popped.begin;
        p_end = popped.end;
        p_diff = popped.diff;
        p_cb = popped.cb;
        p_data = popped.data;
        tag = popped.tag;
        buffer = popped.buffer;
        disk = popped.disk;
        sp -= 1;
        goto popped;
    } else {
        return res;
    }
}

fn lfs_dir_fetchmatch(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    pair: &[LfsBlock; 2],
    fmask: LfsTag,
    ftag: LfsTag,
    id: Option<&mut u16>,
    cb: Option<unsafe extern "C" fn(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32>,
    data: *mut c_void,
) -> LfsStag {
    // we can find tag very efficiently during a fetch, since we're already
    // scanning the entire directory
    let mut besttag: LfsStag = -1;

    // if either block address is invalid we return LFS_ERR_CORRUPT here,
    // otherwise later writes to the pair could fail
    if lfs.block_count > 0
        && (pair[0] >= lfs.block_count || pair[1] >= lfs.block_count)
    {
        return LfsError::Corrupt as LfsStag;
    }

    // find the block with the most recent revision
    let mut revs: [u32; 2] = [0, 0];
    let mut r = 0;
    for i in 0..2 {
        let err = unsafe {
            lfs_util::lfs_bd_read(
                lfs,
                core::ptr::null_mut(),
                &lfs.rcache,
                core::mem::size_of_val(&revs[i]) as LfsSize,
                pair[i],
                0,
                &mut revs[i] as *mut u32 as *mut c_void,
                core::mem::size_of_val(&revs[i]) as LfsSize,
            )
        };
        revs[i] = lfs_util::lfs_fromle32(revs[i]);
        if err != 0 && err != LfsError::Corrupt as i32 {
            return err as LfsStag;
        }

        if err != LfsError::Corrupt as i32
            && lfs_util::lfs_scmp(revs[i], revs[(i + 1) % 2]) > 0
        {
            r = i;
        }
    }

    dir.pair[0] = pair[(r + 0) % 2];
    dir.pair[1] = pair[(r + 1) % 2];
    dir.rev = revs[(r + 0) % 2];
    dir.off = 0; // nonzero = found some commits

    // now scan tags to fetch the actual dir and find possible match
    for i in 0..2 {
        let mut off: LfsOff = 0;
        let mut ptag: LfsTag = 0xffffffff;

        let mut tempcount = 0;
        let mut temptail: [LfsBlock; 2] = [LFS_BLOCK_NULL, LFS_BLOCK_NULL];
        let mut tempsplit = false;
        let mut tempbesttag = besttag;

        // assume not erased until proven otherwise
        let mut maybeerased = false;
        let mut hasfcrc = false;
        let mut fcrc = LfsFcrc { size: 0, crc: 0 };

        dir.rev = lfs_util::lfs_tole32(dir.rev);
        let mut crc = lfs_util::lfs_crc(0xffffffff, unsafe {
            core::slice::from_raw_parts(&dir.rev as *const u32 as *const u8, core::mem::size_of::<u32>())
        });
        dir.rev = lfs_util::lfs_fromle32(dir.rev);

        loop {
            // extract next tag
            let mut tag: LfsTag = 0;
            off += lfs_util::lfs_tag_dsize(ptag);
            let err = unsafe {
                lfs_util::lfs_bd_read(
                    lfs,
                    core::ptr::null_mut(),
                    &lfs.rcache,
                    lfs.cfg.as_ref().unwrap().block_size,
                    dir.pair[0],
                    off,
                    &mut tag as *mut LfsTag as *mut c_void,
                    core::mem::size_of::<LfsTag>() as LfsSize,
                )
            };
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    // can't continue?
                    break;
                }
                return err as LfsStag;
            }

            crc = lfs_util::lfs_crc(crc, unsafe {
                core::slice::from_raw_parts(&tag as *const LfsTag as *const u8, core::mem::size_of::<LfsTag>())
            });
            tag = lfs_util::lfs_frombe32(tag) ^ ptag;

            // next commit not yet programmed?
            if !lfs_util::lfs_tag_isvalid(tag) {
                // we only might be erased if the last tag was a crc
                maybeerased = (lfs_util::lfs_tag_type2(ptag) == LfsType::CCrc as u32);
                break;
            // out of range?
            } else if off + lfs_util::lfs_tag_dsize(tag) > unsafe { (*lfs.cfg).block_size } {
                break;
            }

            ptag = tag;

            if lfs_util::lfs_tag_type2(tag) == LfsType::CCrc as u32 {
                // check the crc attr
                let mut dcrc: u32 = 0;
                let err = unsafe {
                    lfs_util::lfs_bd_read(
                        lfs,
                        core::ptr::null_mut(),
                        &lfs.rcache,
                        (*lfs.cfg).block_size,
                        dir.pair[0],
                        off + core::mem::size_of::<LfsTag>() as LfsOff,
                        &mut dcrc as *mut u32 as *mut c_void,
                        core::mem::size_of::<u32>() as LfsSize,
                    )
                };
                if err != 0 {
                    if err == LfsError::Corrupt as i32 {
                        break;
                    }
                    return err as LfsStag;
                }
                dcrc = lfs_util::lfs_fromle32(dcrc);

                if crc != dcrc {
                    break;
                }

                // reset the next bit if we need to
                ptag ^= ((lfs_util::lfs_tag_chunk(tag) & 1) as LfsTag) << 31;

                // toss our crc into the filesystem seed for
                // pseudorandom numbers, note we use another crc here
                // as a collection function because it is sufficiently
                // random and convenient
                lfs.seed = lfs_util::lfs_crc(lfs.seed, unsafe {
                    core::slice::from_raw_parts(&crc as *const u32 as *const u8, core::mem::size_of::<u32>())
                });

                // update with what's found so far
                besttag = tempbesttag;
                dir.off = off + lfs_util::lfs_tag_dsize(tag);
                dir.etag = ptag;
                dir.count = tempcount;
                dir.tail[0] = temptail[0];
                dir.tail[1] = temptail[1];
                dir.split = tempsplit;

                // reset crc, hasfcrc
                crc = 0xffffffff;
                continue;
            }

            // crc the entry first, hopefully leaving it in the cache
            let err = unsafe {
                lfs_util::lfs_bd_crc(
                    lfs,
                    core::ptr::null_mut(),
                    &lfs.rcache,
                    (*lfs.cfg).block_size,
                    dir.pair[0],
                    off + core::mem::size_of::<LfsTag>() as LfsOff,
                    lfs_util::lfs_tag_dsize(tag) - core::mem::size_of::<LfsTag>() as LfsSize,
                    &mut crc,
                )
            };
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    break;
                }
                return err as LfsStag;
            }

            // directory modification tags?
            if lfs_util::lfs_tag_type1(tag) == LfsType::Name as u32 {
                // increase count of files if necessary
                if lfs_util::lfs_tag_id(tag) >= tempcount {
                    tempcount = lfs_util::lfs_tag_id(tag) + 1;
                }
            } else if lfs_util::lfs_tag_type1(tag) == LfsType::Splice as u32 {
                tempcount += lfs_util::lfs_tag_splice(tag);

                if tag == (lfs_util::LFS_MKTAG(LfsType::Delete as u32, 0, 0) |
                        (lfs_util::LFS_MKTAG(0, 0x3ff, 0) & tempbesttag as u32)) {
                    tempbesttag |= 0x80000000;
                } else if tempbesttag != -1 &&
                        lfs_util::lfs_tag_id(tag) <= lfs_util::lfs_tag_id(tempbesttag as u32) {
                    tempbesttag += lfs_util::LFS_MKTAG(0, lfs_util::lfs_tag_splice(tag), 0) as LfsStag;
                }
            } else if lfs_util::lfs_tag_type1(tag) == LfsType::Tail as u32 {
                tempsplit = (lfs_util::lfs_tag_chunk(tag) & 1) != 0;

                let err = unsafe {
                    lfs_util::lfs_bd_read(
                        lfs,
                        core::ptr::null_mut(),
                        &lfs.rcache,
                        (*lfs.cfg).block_size,
                        dir.pair[0],
                        off + core::mem::size_of::<LfsTag>() as LfsOff,
                        &mut temptail as *mut [LfsBlock; 2] as *mut c_void,
                        8,
                    )
                };
                if err != 0 {
                    if err == LfsError::Corrupt as i32 {
                        break;
                    }
                    return err as LfsStag;
                }
                lfs_util::lfs_pair_fromle32(&mut temptail);
            } else if lfs_util::lfs_tag_type3(tag) == LfsType::FCrc as u32 {
                let err = unsafe {
                    lfs_util::lfs_bd_read(
                        lfs,
                        core::ptr::null_mut(),
                        &lfs.rcache,
                        (*lfs.cfg).block_size,
                        dir.pair[0],
                        off + core::mem::size_of::<LfsTag>() as LfsOff,
                        &mut fcrc as *mut LfsFcrc as *mut c_void,
                        core::mem::size_of::<LfsFcrc>() as LfsSize,
                    )
                };
                if err != 0 {
                    if err == LfsError::Corrupt as i32 {
                        break;
                    }
                    return err as LfsStag;
                }

                lfs_util::lfs_fcrc_fromle32(&mut fcrc);
                hasfcrc = true;
            }

            // found a match for our fetcher?
            if (fmask & tag) == (fmask & ftag) {
                if let Some(callback) = cb {
                    let diskoff = LfsDiskoff {
                        block: dir.pair[0],
                        off: off + core::mem::size_of::<LfsTag>() as LfsOff,
                    };
                    
                    let res = unsafe { callback(data, tag, &diskoff as *const LfsDiskoff as *const c_void) };
                    if res < 0 {
                        if res == LfsError::Corrupt as i32 {
                            break;
                        }
                        return res as LfsStag;
                    }

                    if res == LfsCmp::Eq as i32 {
                        // found a match
                        tempbesttag = tag as LfsStag;
                    } else if ((0x7ff << 20) | (0x3ff << 0)) & tag ==
                            ((0x7ff << 20) | (0x3ff << 0)) & tempbesttag as u32 {
                        // found an identical tag, but contents didn't match
                        // this must mean that our besttag has been overwritten
                        tempbesttag = -1;
                    } else if res == LfsCmp::Gt as i32 &&
                            lfs_util::lfs_tag_id(tag) <= lfs_util::lfs_tag_id(tempbesttag as u32) {
                        // found a greater match, keep track to keep things sorted
                        tempbesttag = tag as LfsStag | 0x80000000;
                    }
                }
            }
        }

        // found no valid commits?
        if dir.off == 0 {
            // try the other block?
            lfs_util::lfs_pair_swap(&mut dir.pair);
            dir.rev = revs[(r+1)%2];
            continue;
        }

        // did we end on a valid commit? we may have an erased block
        dir.erased = false;
        if maybeerased && dir.off % unsafe { (*lfs.cfg).prog_size } == 0 {
            #[cfg(feature = "lfs-multiversion")]
            {
                // note versions < lfs2.1 did not have fcrc tags, if
                // we're < lfs2.1 treat missing fcrc as erased data
                //
                // we don't strictly need to do this, but otherwise writing
                // to lfs2.0 disks becomes very inefficient
                if lfs_util::lfs_fs_disk_version(lfs) < 0x00020001 {
                    dir.erased = true;
                } else if hasfcrc {
                    // do the rest of the check
                } else {
                    // skip the rest of the check
                }
            }

            if hasfcrc {
                // check for an fcrc matching the next prog's erased state, if
                // this failed most likely a previous prog was interrupted, we
                // need a new erase
                let mut fcrc_: u32 = 0xffffffff;
                let err = unsafe {
                    lfs_util::lfs_bd_crc(
                        lfs,
                        core::ptr::null_mut(),
                        &lfs.rcache,
                        (*lfs.cfg).block_size,
                        dir.pair[0],
                        dir.off,
                        fcrc.size,
                        &mut fcrc_,
                    )
                };
                if err != 0 && err != LfsError::Corrupt as i32 {
                    return err as LfsStag;
                }

                // found beginning of erased part?
                dir.erased = (fcrc_ == fcrc.crc);
            }
        }

        // synthetic move
        if lfs_util::lfs_gstate_hasmovehere(&lfs.gdisk, &dir.pair) {
            if lfs_util::lfs_tag_id(lfs.gdisk.tag) == lfs_util::lfs_tag_id(besttag as u32) {
                besttag |= 0

/// 获取目录，使用默认的匹配参数
/// 
/// 这是一个对lfs_dir_fetchmatch的简单包装，设置掩码和标签为-1
/// 以便永远不会匹配到任何标签（因为这种模式设置了无效位）
fn lfs_dir_fetch(lfs: *mut Lfs, dir: *mut LfsMdir, pair: &[LfsBlock; 2]) -> i32 {
    // 注意，mask=-1, tag=-1 永远不会匹配任何标签，因为这个模式设置了无效位
    lfs_dir_fetchmatch(
        lfs, 
        dir, 
        pair,
        !0 as LfsTag,  // (lfs_tag_t)-1
        !0 as LfsTag,  // (lfs_tag_t)-1
        std::ptr::null(), 
        std::ptr::null(), 
        std::ptr::null()
    ) as i32
}

/// 从目录获取全局状态
fn lfs_dir_getgstate(lfs: &mut Lfs, dir: &LfsMdir, gstate: &mut LfsGstate) -> LfsSsize {
    let mut temp = LfsGstate::default();
    let res = lfs_dir_get(
        lfs,
        dir,
        lfs_util::lfs_mktag(0x7ff, 0, 0),
        lfs_util::lfs_mktag(LfsType::MoveState as u16, 0, core::mem::size_of::<LfsGstate>() as u8),
        &mut temp as *mut _ as *mut c_void,
    );
    
    if res < 0 && res != LfsError::NoEnt as i32 {
        return res;
    }

    if res != LfsError::NoEnt as i32 {
        // xor together to find resulting gstate
        lfs_gstate_fromle32(&mut temp);
        lfs_gstate_xor(gstate, &temp);
    }

    0
}

/// 获取目录中特定id的条目信息
/// 
/// # 参数
/// 
/// * `lfs` - 文件系统指针
/// * `dir` - 目录指针
/// * `id` - 条目ID
/// * `info` - 要填充的信息结构体
/// 
/// # 返回值
/// 
/// 成功时返回0，失败时返回负数错误码
fn lfs_dir_getinfo(lfs: &mut Lfs, dir: &mut LfsMdir, 
                  id: u16, info: &mut LfsInfo) -> i32 {
    if id == 0x3ff {
        // 根目录的特殊情况
        info.name[0..1].copy_from_slice(b"/");
        info.name[1] = 0; // 确保null终止
        info.type_ = LfsType::Dir as u8;
        return 0;
    }

    // 获取名称信息
    let tag = lfs_dir_get(
        lfs, 
        dir, 
        LFS_MKTAG(0x780, 0x3ff, 0),
        LFS_MKTAG(LfsType::Name as u16, id, lfs.name_max + 1), 
        &mut info.name as *mut _ as *mut c_void
    );
    
    if tag < 0 {
        return tag;
    }

    info.type_ = lfs_tag_type3(tag as u32) as u8;

    // 获取结构信息
    let mut ctz = LfsCtz { head: 0, size: 0 };
    let tag = lfs_dir_get(
        lfs, 
        dir, 
        LFS_MKTAG(0x700, 0x3ff, 0),
        LFS_MKTAG(LfsType::Struct as u16, id, core::mem::size_of::<LfsCtz>() as u16), 
        &mut ctz as *mut _ as *mut c_void
    );
    
    if tag < 0 {
        return tag;
    }
    
    lfs_ctz_fromle32(&mut ctz);

    // 设置大小信息
    let tag_type = lfs_tag_type3(tag as u32);
    if tag_type == LfsType::CtzStruct as u16 {
        info.size = ctz.size;
    } else if tag_type == LfsType::InlineStruct as u16 {
        info.size = lfs_tag_size(tag as u32) as u32;
    } else {
        // 如果是其他类型，保持size默认值
    }

    0
}

/// 比较目录项名称与给定名称
/// 
/// 用于在目录中查找特定名称的项目
fn lfs_dir_find_match(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32 {
    let name = unsafe { &*(data as *const LfsDirFindMatch) };
    let lfs = unsafe { &mut *name.lfs };
    let disk = unsafe { &*(buffer as *const LfsDiskoff) };

    // 比较与磁盘上的内容
    let diff = lfs_util::lfs_min(name.size, lfs_tag_size(tag));
    let res = lfs_bd_cmp(
        lfs,
        core::ptr::null_mut(),
        &lfs.rcache,
        diff,
        disk.block,
        disk.off,
        name.name,
        diff,
    );
    if res != LfsCmp::Eq as i32 {
        return res;
    }

    // 只有当我们的大小仍然相同时才相等
    if name.size != lfs_tag_size(tag) {
        return if name.size < lfs_tag_size(tag) {
            LfsCmp::Lt as i32
        } else {
            LfsCmp::Gt as i32
        };
    }

    // 找到匹配项！
    LfsCmp::Eq as i32
}

fn lfs_dir_find(lfs: &mut Lfs, dir: &mut LfsMdir, path: &mut &[u8], id: &mut u16) -> LfsStag {
    // we reduce path to a single name if we can find it
    let mut name = *path;

    // default to root dir
    let mut tag: LfsStag = lfs_tag::lfs_mktag(LfsType::Dir as u16, 0x3ff, 0) as LfsStag;
    dir.tail[0] = lfs.root[0];
    dir.tail[1] = lfs.root[1];

    // empty paths are not allowed
    if name.is_empty() {
        return LfsError::Inval as LfsStag;
    }

    loop {
        'nextname: loop {
            // skip slashes if we're a directory
            if lfs_tag::lfs_tag_type3(tag as LfsTag) == LfsType::Dir as u16 {
                name = name.trim_start_matches(|c| c == b'/');
            }
            let namelen = name.iter().position(|&c| c == b'/').unwrap_or(name.len());

            // skip '.'
            if namelen == 1 && name[0] == b'.' {
                name = &name[namelen..];
                continue 'nextname;
            }

            // error on unmatched '..', trying to go above root?
            if namelen == 2 && name[0] == b'.' && name[1] == b'.' {
                return LfsError::Inval as LfsStag;
            }

            // skip if matched by '..' in name
            let mut suffix = &name[namelen..];
            let mut depth = 1;
            loop {
                suffix = suffix.trim_start_matches(|c| c == b'/');
                let sufflen = suffix.iter().position(|&c| c == b'/').unwrap_or(suffix.len());
                if sufflen == 0 {
                    break;
                }

                if sufflen == 1 && suffix[0] == b'.' {
                    // noop
                } else if sufflen == 2 && suffix[0] == b'.' && suffix[1] == b'.' {
                    depth -= 1;
                    if depth == 0 {
                        name = &suffix[sufflen..];
                        continue 'nextname;
                    }
                } else {
                    depth += 1;
                }

                suffix = &suffix[sufflen..];
            }

            // found path
            if name.is_empty() {
                return tag;
            }

            // update what we've found so far
            *path = name;

            // only continue if we're a directory
            if lfs_tag::lfs_tag_type3(tag as LfsTag) != LfsType::Dir as u16 {
                return LfsError::NotDir as LfsStag;
            }

            // grab the entry data
            if lfs_tag::lfs_tag_id(tag as LfsTag) != 0x3ff {
                let res = lfs_dir::lfs_dir_get(
                    lfs,
                    dir,
                    lfs_tag::lfs_mktag(LfsType::Globals as u16, 0x3ff, 0),
                    lfs_tag::lfs_mktag(LfsType::Struct as u16, lfs_tag::lfs_tag_id(tag as LfsTag), 8),
                    dir.tail.as_mut_ptr() as *mut c_void,
                );
                if res < 0 {
                    return res;
                }
                lfs_pair::lfs_pair_fromle32(&mut dir.tail);
            }

            // find entry matching name
            loop {
                tag = lfs_dir::lfs_dir_fetchmatch(
                    lfs,
                    dir,
                    &dir.tail,
                    lfs_tag::lfs_mktag(0x780, 0, 0),
                    lfs_tag::lfs_mktag(LfsType::Name as u16, 0, namelen as u32),
                    id,
                    lfs_dir::lfs_dir_find_match,
                    &mut LfsDirFindMatch {
                        lfs,
                        name: name.as_ptr() as *const c_void,
                        size: namelen as LfsSize,
                    } as *mut _ as *mut c_void,
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

            // to next name
            name = &name[namelen..];
            break;
        }
    }
}

/// 将数据提交到目录中，同时更新CRC值和偏移量
fn lfs_dir_commitprog(lfs: &mut Lfs, commit: &mut LfsCommit, buffer: *const c_void, size: LfsSize) -> i32 {
    let err = lfs_bd_prog(
        lfs,
        &mut lfs.pcache,
        &mut lfs.rcache,
        false,
        commit.block,
        commit.off,
        buffer as *const u8,
        size
    );
    
    if err != 0 {
        return err;
    }

    commit.crc = lfs_crc(commit.crc, buffer, size);
    commit.off += size;
    0
}

/// 将属性提交到目录
/// 
/// `lfs`: 文件系统
/// `commit`: 提交状态
/// `tag`: 要写入的标签
/// `buffer`: 要写入的数据
/// 
/// 返回: 0表示成功，负值表示错误
fn lfs_dir_commitattr(lfs: &mut Lfs, commit: &mut LfsCommit, 
                     tag: LfsTag, buffer: *const c_void) -> i32 {
    // 检查是否有足够空间
    let dsize = lfs_util::lfs_tag_dsize(tag);
    if commit.off + dsize > commit.end {
        return LfsError::NoSpc as i32;
    }

    // 写出标签
    let ntag = lfs_util::lfs_tobe32((tag & 0x7fffffff) ^ commit.ptag);
    let err = lfs_dir_commitprog(lfs, commit, &ntag as *const _ as *const c_void, 
                                 core::mem::size_of::<LfsTag>() as LfsSize);
    if err != 0 {
        return err;
    }

    if (tag & 0x80000000) == 0 {
        // 从内存写入
        let err = lfs_dir_commitprog(lfs, commit, buffer, 
                                     dsize - core::mem::size_of::<LfsTag>() as LfsSize);
        if err != 0 {
            return err;
        }
    } else {
        // 从磁盘写入
        let disk = buffer as *const LfsDiskoff;
        let disk = unsafe { &*disk };
        
        for i in 0..dsize - core::mem::size_of::<LfsTag>() as LfsSize {
            // 依靠缓存使其高效
            let mut dat: u8 = 0;
            let err = lfs_bd_read(lfs, 
                                 core::ptr::null_mut(), 
                                 &lfs.rcache,
                                 dsize - core::mem::size_of::<LfsTag>() as LfsSize - i,
                                 disk.block, 
                                 disk.off + i, 
                                 &mut dat as *mut u8 as *mut c_void, 
                                 1);
            if err != 0 {
                return err;
            }

            let err = lfs_dir_commitprog(lfs, commit, 
                                        &dat as *const u8 as *const c_void, 1);
            if err != 0 {
                return err;
            }
        }
    }

    commit.ptag = tag & 0x7fffffff;
    0
}

/// 提交目录的CRC校验和
fn lfs_dir_commitcrc(lfs: *mut Lfs, commit: *mut LfsCommit) -> i32 {
    unsafe {
        // 对齐到程序单元
        //
        // 这变得有点复杂，因为我们有两种CRC类型：
        // - 带fcrc的5字CRC，用于检查后续程序（块的中间）
        // - 不带后续程序的2字CRC（块的末尾）
        let end = lfs_util::lfs_alignup(
            lfs_util::lfs_min(
                (*commit).off + 5 * core::mem::size_of::<u32>() as LfsOff,
                (*(*lfs).cfg).block_size as LfsOff
            ),
            (*(*lfs).cfg).prog_size as LfsOff
        );

        let mut off1: LfsOff = 0;
        let mut crc1: u32 = 0;

        // 创建CRC标签以填充提交的剩余部分，注意填充部分不计算CRC
        // 这允许获取时跳过填充部分，但使提交稍微复杂一些
        while (*commit).off < end {
            let mut noff = (
                lfs_util::lfs_min(end - ((*commit).off + core::mem::size_of::<LfsTag>() as LfsOff), 0x3fe)
                + ((*commit).off + core::mem::size_of::<LfsTag>() as LfsOff)
            );
            // 对于CRC标签太大了？需要填充提交
            if noff < end {
                noff = lfs_util::lfs_min(noff, end - 5 * core::mem::size_of::<u32>() as LfsOff);
            }

            // 有空间给fcrc吗？
            let mut eperturb: u8 = 0xff;
            if noff >= end && noff <= (*(*lfs).cfg).block_size as LfsOff - (*(*lfs).cfg).prog_size as LfsOff {
                // 首先读取前导字节，这总是包含一个位
                // 我们可以扰动它以避免不改变fcrc的写入
                let err = lfs_util::lfs_bd_read(
                    lfs,
                    core::ptr::null_mut(),
                    &mut (*lfs).rcache,
                    (*(*lfs).cfg).prog_size,
                    (*commit).block,
                    noff,
                    &mut eperturb as *mut u8 as *mut c_void,
                    1
                );
                if err != 0 && err != LfsError::Corrupt as i32 {
                    return err;
                }

                #[cfg(feature = "LFS_MULTIVERSION")]
                {
                    // 不幸的是，fcrc破坏了< lfs2.1的mdir获取，所以只有当我们是
                    // >= lfs2.1的文件系统时才写入这些
                    if lfs_util::lfs_fs_disk_version(lfs) <= 0x00020000 {
                        // 不写入fcrc
                    } else {
                        // 寻找预期的fcrc，不要费心避免重读
                        // eperturb，它应该仍在我们的缓存中
                        let mut fcrc = LfsFcrc {
                            size: (*(*lfs).cfg).prog_size,
                            crc: 0xffffffff
                        };
                        let err = lfs_util::lfs_bd_crc(
                            lfs,
                            core::ptr::null_mut(),
                            &mut (*lfs).rcache,
                            (*(*lfs).cfg).prog_size,
                            (*commit).block,
                            noff,
                            fcrc.size,
                            &mut fcrc.crc
                        );
                        if err != 0 && err != LfsError::Corrupt as i32 {
                            return err;
                        }

                        lfs_util::lfs_fcrc_tole32(&mut fcrc);
                        let err = lfs_util::lfs_dir_commitattr(
                            lfs,
                            commit,
                            lfs_util::LFS_MKTAG(LfsType::FCrc as u16, 0x3ff, core::mem::size_of::<LfsFcrc>() as u16),
                            &fcrc as *const _ as *const c_void
                        );
                        if err != 0 {
                            return err;
                        }
                    }
                }

                #[cfg(not(feature = "LFS_MULTIVERSION"))]
                {
                    // 寻找预期的fcrc，不要费心避免重读
                    // eperturb，它应该仍在我们的缓存中
                    let mut fcrc = LfsFcrc {
                        size: (*(*lfs).cfg).prog_size,
                        crc: 0xffffffff
                    };
                    let err = lfs_util::lfs_bd_crc(
                        lfs,
                        core::ptr::null_mut(),
                        &mut (*lfs).rcache,
                        (*(*lfs).cfg).prog_size,
                        (*commit).block,
                        noff,
                        fcrc.size,
                        &mut fcrc.crc
                    );
                    if err != 0 && err != LfsError::Corrupt as i32 {
                        return err;
                    }

                    lfs_util::lfs_fcrc_tole32(&mut fcrc);
                    let err = lfs_util::lfs_dir_commitattr(
                        lfs,
                        commit,
                        lfs_util::LFS_MKTAG(LfsType::FCrc as u16, 0x3ff, core::mem::size_of::<LfsFcrc>() as u16),
                        &fcrc as *const _ as *const c_void
                    );
                    if err != 0 {
                        return err;
                    }
                }
            }

            // 构建提交CRC
            let mut ccrc = struct_anon {
                tag: 0_u32,
                crc: 0_u32,
            };
            let ntag = lfs_util::LFS_MKTAG(
                LfsType::CCrc as u16 + (((!eperturb) >> 7) as u16),
                0x3ff,
                (noff - ((*commit).off + core::mem::size_of::<LfsTag>() as LfsOff)) as u16
            );
            ccrc.tag = lfs_util::lfs_tobe32(ntag ^ (*commit).ptag);
            (*commit).crc = lfs_util::lfs_crc((*commit).crc, &ccrc.tag, core::mem::size_of::<LfsTag>());
            ccrc.crc = lfs_util::lfs_tole32((*commit).crc);

            let err = lfs_util::lfs_bd_prog(
                lfs,
                &mut (*lfs).pcache,
                &mut (*lfs).rcache,
                false,
                (*commit).block,
                (*commit).off,
                &ccrc as *const _ as *const c_void,
                core::mem::size_of::<struct_anon>()
            );
            if err != 0 {
                return err;
            }

            // 跟踪非填充校验和以进行验证
            if off1 == 0 {
                off1 = (*commit).off + core::mem::size_of::<LfsTag>() as LfsOff;
                crc1 = (*commit).crc;
            }

            (*commit).off = noff;
            // 扰动有效位？
            (*commit).ptag = ntag ^ ((0x80u32 & !eperturb as u32) << 24);
            // 重置crc为下一个提交
            (*commit).crc = 0xffffffff;

            // 手动刷新，因为我们不编程填充部分，这会使缓存层混淆
            if noff >= end || noff >= (*lfs).pcache.off + (*(*lfs).cfg).cache_size as LfsOff {
                // 刷新缓冲区
                let err = lfs_util::lfs_bd_sync(lfs, &mut (*lfs).pcache, &mut (*lfs).rcache, false);
                if err != 0 {
                    return err;
                }
            }
        }

        // 成功提交，检查校验和以确保
        //
        // 注意，我们不需要检查填充提交，最坏情况下
        // 如果它们被损坏，我们也会进行压缩
        let off = (*commit).begin;
        let mut crc: u32 = 0xffffffff;
        let err = lfs_util::lfs_bd_crc(
            lfs,
            core::ptr::null_mut(),
            &mut (*lfs).rcache,
            off1 + core::mem::size_of::<u32>() as LfsOff,
            (*commit).block,
            off,
            off1 - off,
            &mut crc
        );
        if err != 0 {
            return err;
        }

        // 检查已知CRC的非填充提交
        if crc != crc1 {
            return LfsError::Corrupt as i32;
        }

        // 确保检查CRC，以防万一我们碰巧拾取
        // 不相关的CRC（冻结块？）
        let err = lfs_util::lfs_bd_crc(
            lfs,
            core::ptr::null_mut(),
            &mut (*lfs).rcache,
            core::mem::size_of::<u32>() as LfsSize,
            (*commit).block,
            off1,
            core::mem::size_of::<u32>() as LfsSize,
            &mut crc
        );
        if err != 0 {
            return err;
        }

        if crc != 0 {
            return LfsError::Corrupt as i32;
        }

        0
    }
}

// 用于ccrc的匿名结构
#[derive(Clone, Copy)]
#[repr(C)]
struct struct_anon {
    tag: u32,
    crc: u32,
}

/// 为目录分配一对块
fn lfs_dir_alloc(lfs: &mut Lfs, dir: &mut LfsMdir) -> i32 {
    // 分配目录块对（逆序，所以我们先写块1）
    for i in 0..2 {
        let err = lfs_alloc(lfs, &mut dir.pair[(i+1)%2]);
        if err != 0 {
            return err;
        }
    }

    // 为了可重现性，如果初始块不可读，将修订设置为0
    dir.rev = 0;

    // 而不是覆盖其中一个块，我们只是假设修订可能有效
    let mut rev = 0u32;
    let err = lfs_bd_read(
        lfs,
        None,
        &lfs.rcache,
        core::mem::size_of::<u32>() as LfsSize,
        dir.pair[0],
        0,
        &mut rev as *mut u32 as *mut c_void,
        core::mem::size_of::<u32>() as LfsSize
    );
    dir.rev = lfs_util::lfs_fromle32(rev);
    if err != 0 && err != LfsError::Corrupt as i32 {
        return err;
    }

    // 为确保不会立即驱逐，将新修订计数对齐到我们的block_cycles模数
    // 参见lfs_dir_compact了解为什么我们的模数这样调整
    if unsafe { (*lfs.cfg).block_cycles > 0 } {
        dir.rev = lfs_util::lfs_alignup(dir.rev, ((unsafe { (*lfs.cfg).block_cycles } + 1) | 1) as u32);
    }

    // 设置默认值
    dir.off = core::mem::size_of::<u32>() as LfsOff;
    dir.etag = 0xffffffff;
    dir.count = 0;
    dir.tail[0] = LFS_BLOCK_NULL;
    dir.tail[1] = LFS_BLOCK_NULL;
    dir.erased = false;
    dir.split = false;

    // 先不写出，让调用者处理
    0
}

/// Drop a directory, this eats all pins and directory traversals
/// that pass through this directory. This operation is kind of complex,
/// we must be sure to pass through the old directory while collecting
/// the new state, and update the root as we go.
///
/// 丢弃目录，这会销毁所有通过此目录的引脚和目录遍历。
/// 此操作相当复杂，我们必须确保在收集新状态时通过旧目录，并在前进时更新根目录。
fn lfs_dir_drop(lfs: &mut Lfs, dir: &mut LfsMdir, tail: &mut LfsMdir) -> LfsError {
    // steal state
    let err = lfs_dir_getgstate(lfs, tail, &mut lfs.gdelta);
    if err != LfsError::Ok {
        return err;
    }

    // steal tail
    lfs_pair_tole32(&mut tail.tail);
    let err = lfs_dir_commit(
        lfs,
        dir,
        &[LfsMattr {
            tag: LFS_MKTAG(
                LfsType::Tail as u16 + (tail.split as u16),
                0x3ff,
                8
            ),
            buffer: tail.tail.as_ptr() as *const c_void,
        }]
    );
    lfs_pair_fromle32(&mut tail.tail);
    if err != LfsError::Ok {
        return err;
    }

    LfsError::Ok
}

fn lfs_dir_split(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    attrs: *const LfsMattr,
    attrcount: i32,
    source: &mut LfsMdir,
    split: u16,
    end: u16,
) -> i32 {
    // create tail metadata pair
    let mut tail = LfsMdir::default();
    let err = lfs_dir_alloc(lfs, &mut tail);
    if err != 0 {
        return err;
    }

    tail.split = dir.split;
    tail.tail[0] = dir.tail[0];
    tail.tail[1] = dir.tail[1];

    // note we don't care about LFS_OK_RELOCATED
    let res = lfs_dir_compact(lfs, &mut tail, attrs, attrcount, source, split, end);
    if res < 0 {
        return res;
    }

    dir.tail[0] = tail.pair[0];
    dir.tail[1] = tail.pair[1];
    dir.split = true;

    // update root if needed
    if lfs_pair_cmp(dir.pair, lfs.root) == 0 && split == 0 {
        lfs.root[0] = tail.pair[0];
        lfs.root[1] = tail.pair[1];
    }

    0
}

fn lfs_dir_commit_size(p: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32 {
    let size = unsafe { &mut *(p as *mut LfsSize) };
    let _ = buffer; // 忽略未使用的参数

    // 增加size值，加上标签中指定的数据大小
    *size += lfs_tag_dsize(tag);
    0
}

/// Commit helper function for directory entries
unsafe fn lfs_dir_commit_commit(p: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32 {
    let commit = p as *mut LfsDirCommitCommit;
    lfs_dir_commitattr((*commit).lfs, (*commit).commit, tag, buffer)
}

/// 检查目录是否需要重定位（用于磨损均衡）
fn lfs_dir_needsrelocation(lfs: &Lfs, dir: &LfsMdir) -> bool {
    // 如果我们的修订计数 == n * block_cycles，我们应该强制进行重定位，
    // 这是littlefs在元数据对级别进行磨损均衡的方式。注意，我们
    // 实际上使用(block_cycles+1)|1，这是为了避免两种极端情况：
    // 1. block_cycles = 1，这将阻止重定位终止
    // 2. block_cycles = 2n，由于别名，只会重定位对中的一个元数据块，
    //    从而使这一功能实际上毫无用处
    unsafe {
        (*lfs.cfg).block_cycles > 0
            && ((dir.rev + 1) % ((((*lfs.cfg).block_cycles + 1) | 1) as u32) == 0)
    }
}

/// 压缩目录，将源目录中的条目复制到目标目录
fn lfs_dir_compact(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    attrs: *const LfsMattr,
    attrcount: i32,
    source: &LfsMdir,
    begin: u16,
    end: u16,
) -> i32 {
    // 保存一些状态以防块损坏
    let mut relocated = false;
    let mut tired = lfs_dir_needsrelocation(lfs, dir);

    // 增加修订计数
    dir.rev += 1;

    // 在迁移期间不要主动重新定位块，这可能导致许多失败状态
    // 例如：如果我们重新定位根目录，可能会破坏v1超级块，
    // 如果我们重新定位目录的头部，可能会使目录指针无效。
    // 此外，重新定位增加了lfs_migration的整体复杂性，这已经是一个复杂的操作。
    #[cfg(feature = "LFS_MIGRATE")]
    if !lfs.lfs1.is_null() {
        tired = false;
    }

    if tired && lfs_pair_cmp(dir.pair, &[0, 1]) != 0 {
        // 我们写入太多，是时候重新定位了
        goto_relocate!(relocate);
    }

    // 开始循环，提交压缩到块，直到成功
    loop {
        {
            // 设置提交状态
            let mut commit = LfsCommit {
                block: dir.pair[1],
                off: 0,
                ptag: 0xffffffff,
                crc: 0xffffffff,
                begin: 0,
                end: (if lfs.cfg.is_null() || unsafe { (*lfs.cfg).metadata_max } == 0 { 
                    unsafe { (*lfs.cfg).block_size }
                } else {
                    unsafe { (*lfs.cfg).metadata_max }
                }) - 8,
            };

            // 擦除要写入的块
            let err = lfs_bd_erase(lfs, dir.pair[1]);
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    goto_relocate!(relocate);
                }
                return err;
            }

            // 写出头部
            dir.rev = lfs_util::lfs_tole32(dir.rev);
            let err = lfs_dir_commitprog(lfs, &mut commit, 
                &dir.rev as *const u32 as *const c_void, core::mem::size_of::<u32>() as LfsSize);
            dir.rev = lfs_util::lfs_fromle32(dir.rev);
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    goto_relocate!(relocate);
                }
                return err;
            }

            // 遍历目录，这次写出所有唯一标签
            let mut commit_data = LfsDirCommitCommit {
                lfs,
                commit: &mut commit,
            };
            let err = lfs_dir_traverse(
                lfs,
                source,
                0,
                0xffffffff,
                attrs,
                attrcount,
                LFS_MKTAG!(0x400, 0x3ff, 0),
                LFS_MKTAG!(LfsType::Name as u32, 0, 0),
                begin,
                end,
                -begin as i16,
                Some(lfs_dir_commit_commit),
                &mut commit_data as *mut _ as *mut c_void,
            );
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    goto_relocate!(relocate);
                }
                return err;
            }

            // 提交尾部，该尾部可能在最后一次大小检查后是新的
            if !lfs_pair_isnull(dir.tail) {
                lfs_pair_tole32(dir.tail.as_mut_ptr());
                let err = lfs_dir_commitattr(
                    lfs,
                    &mut commit,
                    LFS_MKTAG!(
                        (LfsType::Tail as u32) + (if dir.split { 1 } else { 0 }),
                        0x3ff,
                        8
                    ),
                    dir.tail.as_ptr() as *const c_void,
                );
                lfs_pair_fromle32(dir.tail.as_mut_ptr());
                if err != 0 {
                    if err == LfsError::Corrupt as i32 {
                        goto_relocate!(relocate);
                    }
                    return err;
                }
            }

            // 带入gstate?
            let mut delta = LfsGstate { tag: 0, pair: [0, 0] };
            if !relocated {
                lfs_gstate_xor(&mut delta, &lfs.gdisk);
                lfs_gstate_xor(&mut delta, &lfs.gstate);
            }
            lfs_gstate_xor(&mut delta, &lfs.gdelta);
            delta.tag &= !LFS_MKTAG!(0, 0, 0x3ff);

            let err = lfs_dir_getgstate(lfs, dir, &delta);
            if err != 0 {
                return err;
            }

            if !lfs_gstate_iszero(&delta) {
                lfs_gstate_tole32(&mut delta);
                let err = lfs_dir_commitattr(
                    lfs,
                    &mut commit,
                    LFS_MKTAG!(LfsType::MoveState as u32, 0x3ff, core::mem::size_of::<LfsGstate>() as u32),
                    &delta as *const LfsGstate as *const c_void,
                );
                if err != 0 {
                    if err == LfsError::Corrupt as i32 {
                        goto_relocate!(relocate);
                    }
                    return err;
                }
            }

            // 用CRC完成提交
            let err = lfs_dir_commitcrc(lfs, &mut commit);
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    goto_relocate!(relocate);
                }
                return err;
            }

            // 成功压缩，交换目录对以表示最新
            debug_assert!(commit.off % unsafe { (*lfs.cfg).prog_size } == 0);
            lfs_pair_swap(&mut dir.pair);
            dir.count = end - begin;
            dir.off = commit.off;
            dir.etag = commit.ptag;
            // 更新gstate
            lfs.gdelta = LfsGstate { tag: 0, pair: [0, 0] };
            if !relocated {
                lfs.gdisk = lfs.gstate.clone();
            }
        }
        break;

        relocate:
        // 提交被损坏，丢弃缓存并准备重新定位块
        relocated = true;
        lfs_cache_drop(lfs, &lfs.pcache);
        if !tired {
            debug!("Bad block at 0x{:x}", dir.pair[1]);
        }

        // 不能重新定位超级块，文件系统现在已冻结
        if lfs_pair_cmp(dir.pair, &[0, 1]) == 0 {
            warn!("Superblock 0x{:x} has become unwritable", dir.pair[1]);
            return LfsError::NoSpc as i32;
        }

        // 重新定位对的一半
        let err = lfs_alloc(lfs, &mut dir.pair[1]);
        if err != 0 && (err != LfsError::NoSpc as i32 || !tired) {
            return err;
        }

        tired = false;
    }

    return if relocated { LfsOk::Relocated as i32 } else { 0 };
}

fn lfs_dir_splittingcompact(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    attrs: &[LfsMattr],
    attrcount: i32,
    source: &LfsMdir,
    begin: u16,
    end: u16,
) -> i32 {
    let mut end = end;
    let begin = begin;

    loop {
        // find size of first split, we do this by halving the split until
        // the metadata is guaranteed to fit
        //
        // Note that this isn't a true binary search, we never increase the
        // split size. This may result in poorly distributed metadata but isn't
        // worth the extra code size or performance hit to fix.
        let mut split = begin;
        while end - split > 1 {
            let mut size: LfsSize = 0;
            let err = lfs_dir_traverse(
                lfs,
                source,
                0,
                0xffffffff,
                attrs,
                attrcount,
                LFS_MKTAG(0x400, 0x3ff, 0),
                LFS_MKTAG(LfsType::Name as u16, 0, 0),
                split,
                end,
                -split as i16,
                lfs_dir_commit_size,
                &mut size as *mut _ as *mut c_void,
            );
            if err != 0 {
                return err;
            }

            // space is complicated, we need room for:
            //
            // - tail:         4+2*4 = 12 bytes
            // - gstate:       4+3*4 = 16 bytes
            // - move delete:  4     = 4 bytes
            // - crc:          4+4   = 8 bytes
            //                 total = 40 bytes
            //
            // And we cap at half a block to avoid degenerate cases with
            // nearly-full metadata blocks.
            //
            let metadata_max = if unsafe { (*lfs.cfg).metadata_max } != 0 {
                unsafe { (*lfs.cfg).metadata_max }
            } else {
                unsafe { (*lfs.cfg).block_size }
            };

            if end - split < 0xff
                && size <= lfs_util::lfs_min(
                    metadata_max - 40,
                    lfs_util::lfs_alignup(
                        metadata_max / 2,
                        unsafe { (*lfs.cfg).prog_size },
                    ),
                )
            {
                break;
            }

            split = split + ((end - split) / 2);
        }

        if split == begin {
            // no split needed
            break;
        }

        // split into two metadata pairs and continue
        let err = lfs_dir_split(lfs, dir, attrs, attrcount, source, split, end);
        if err != 0 && err != LfsError::NoSpc as i32 {
            return err;
        }

        if err != 0 {
            // we can't allocate a new block, try to compact with degraded
            // performance
            LFS_WARN!(
                "Unable to split {{0x{:x}, 0x{:x}}}",
                dir.pair[0],
                dir.pair[1]
            );
            break;
        } else {
            end = split;
        }
    }

    if lfs_dir_needsrelocation(lfs, dir)
        && lfs_pair_cmp(
            &dir.pair,
            &[0 as LfsBlock, 1 as LfsBlock],
        ) == LfsCmp::Eq as i32
    {
        // oh no! we're writing too much to the superblock,
        // should we expand?
        let size = lfs_fs_size_(lfs);
        if size < 0 {
            return size;
        }

        // littlefs cannot reclaim expanded superblocks, so expand cautiously
        //
        // if our filesystem is more than ~88% full, don't expand, this is
        // somewhat arbitrary
        if lfs.block_count - size as LfsSize > lfs.block_count / 8 {
            LFS_DEBUG!("Expanding superblock at rev {}", dir.rev);
            let err = lfs_dir_split(lfs, dir, attrs, attrcount, source, begin, end);
            if err != 0 && err != LfsError::NoSpc as i32 {
                return err;
            }

            if err != 0 {
                // welp, we tried, if we ran out of space there's not much
                // we can do, we'll error later if we've become frozen
                LFS_WARN!("Unable to expand superblock");
            } else {
                // duplicate the superblock entry into the new superblock
                end = 1;
            }
        }
    }

    lfs_dir_compact(lfs, dir, attrs, attrcount, source, begin, end)
}

fn lfs_dir_relocatingcommit(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    pair: &[LfsBlock; 2],
    attrs: &[LfsMattr],
    attrcount: i32,
    pdir: &mut LfsMdir,
) -> i32 {
    let mut state = 0;

    // calculate changes to the directory
    let mut hasdelete = false;
    for i in 0..attrcount as usize {
        if lfs_tag_type3(attrs[i].tag) == LFS_TYPE_CREATE as u32 {
            dir.count += 1;
        } else if lfs_tag_type3(attrs[i].tag) == LFS_TYPE_DELETE as u32 {
            assert!(dir.count > 0);
            dir.count -= 1;
            hasdelete = true;
        } else if lfs_tag_type1(attrs[i].tag) == LFS_TYPE_TAIL as u32 {
            let buffer = attrs[i].buffer as *const LfsBlock;
            unsafe {
                dir.tail[0] = *buffer.offset(0);
                dir.tail[1] = *buffer.offset(1);
            }
            dir.split = (lfs_tag_chunk(attrs[i].tag) & 1) != 0;
            lfs_pair_fromle32(&mut dir.tail);
        }
    }

    // should we actually drop the directory block?
    if hasdelete && dir.count == 0 {
        assert!(!pdir.pair[0] == LFS_BLOCK_NULL);
        let err = lfs_fs_pred(lfs, &dir.pair, pdir);
        if err != 0 && err != LfsError::NoEnt as i32 {
            return err;
        }

        if err != LfsError::NoEnt as i32 && pdir.split {
            state = LfsOk::Dropped as i32;
            goto!(fixmlist);
        }
    }

    if dir.erased {
        // try to commit
        let mut commit = LfsCommit {
            block: dir.pair[0],
            off: dir.off,
            ptag: dir.etag,
            crc: 0xffffffff,

            begin: dir.off,
            end: (if unsafe { (*lfs.cfg).metadata_max } != 0 {
                unsafe { (*lfs.cfg).metadata_max }
            } else {
                unsafe { (*lfs.cfg).block_size }
            }) - 8,
        };

        // traverse attrs that need to be written out
        lfs_pair_tole32(&mut dir.tail);
        let mut commit_data = LfsDirCommitCommit {
            lfs,
            commit: &mut commit,
        };
        let err = lfs_dir_traverse(
            lfs,
            dir,
            dir.off,
            dir.etag,
            attrs,
            attrcount,
            0,
            0,
            0,
            0,
            0,
            Some(lfs_dir_commit_commit),
            &mut commit_data as *mut _ as *mut c_void,
        );
        lfs_pair_fromle32(&mut dir.tail);
        if err != 0 {
            if err == LfsError::NoSpc as i32 || err == LfsError::Corrupt as i32 {
                goto!(compact);
            }
            return err;
        }

        // commit any global diffs if we have any
        let mut delta = LfsGstate { tag: 0, pair: [0, 0] };
        lfs_gstate_xor(&mut delta, &lfs.gstate);
        lfs_gstate_xor(&mut delta, &lfs.gdisk);
        lfs_gstate_xor(&mut delta, &lfs.gdelta);
        delta.tag &= !LFS_MKTAG(0, 0, 0x3ff);
        if !lfs_gstate_iszero(&delta) {
            let err = lfs_dir_getgstate(lfs, dir, &mut delta);
            if err != 0 {
                return err;
            }

            lfs_gstate_tole32(&mut delta);
            let err = lfs_dir_commitattr(
                lfs,
                &mut commit,
                LFS_MKTAG(LFS_TYPE_MOVESTATE as u32, 0x3ff, core::mem::size_of::<LfsGstate>() as u32),
                &delta as *const _ as *const c_void,
            );
            if err != 0 {
                if err == LfsError::NoSpc as i32 || err == LfsError::Corrupt as i32 {
                    goto!(compact);
                }
                return err;
            }
        }

        // finalize commit with the crc
        let err = lfs_dir_commitcrc(lfs, &mut commit);
        if err != 0 {
            if err == LfsError::NoSpc as i32 || err == LfsError::Corrupt as i32 {
                goto!(compact);
            }
            return err;
        }

        // successful commit, update dir
        assert!(commit.off % unsafe { (*lfs.cfg).prog_size } == 0);
        dir.off = commit.off;
        dir.etag = commit.ptag;
        // and update gstate
        lfs.gdisk = lfs.gstate;
        lfs.gdelta = LfsGstate { tag: 0, pair: [0, 0] };

        goto!(fixmlist);
    }

    compact: {
        // fall back to compaction
        lfs_cache_drop(lfs, &mut lfs.pcache);

        state = lfs_dir_splittingcompact(lfs, dir, attrs, attrcount, dir, 0, dir.count as i32);
        if state < 0 {
            return state;
        }

        goto!(fixmlist);
    }

    fixmlist: {
        // this complicated bit of logic is for fixing up any active
        // metadata-pairs that we may have affected
        //
        // note we have to make two passes since the mdir passed to
        // lfs_dir_commit could also be in this list, and even then
        // we need to copy the pair so they don't get clobbered if we refetch
        // our mdir.
        let oldpair = [pair[0], pair[1]];
        let mut mlist = lfs.mlist;
        while !mlist.is_null() {
            if lfs_pair_cmp(unsafe { (*mlist).m.pair }, oldpair) == 0 {
                unsafe { (*mlist).m = *dir; }
                if unsafe { (*mlist).m.pair[0] } != pair[0] || unsafe { (*mlist).m.pair[1] } != pair[1] {
                    for i in 0..attrcount as usize {
                        if lfs_tag_type3(attrs[i].tag) == LFS_TYPE_DELETE as u32 &&
                                unsafe { (*mlist).id } == lfs_tag_id(attrs[i].tag) {
                            unsafe {
                                (*mlist).m.pair[0] = LFS_BLOCK_NULL;
                                (*mlist).m.pair[1] = LFS_BLOCK_NULL;
                            }
                        } else if lfs_tag_type3(attrs[i].tag) == LFS_TYPE_DELETE as u32 &&
                                unsafe { (*mlist).id } > lfs_tag_id(attrs[i].tag) {
                            unsafe { (*mlist).id -= 1; }
                            if unsafe { (*mlist).type_ } == LFS_TYPE_DIR as u8 {
                                let dir_ptr = mlist as *mut LfsDir;
                                unsafe { (*dir_ptr).pos -= 1; }
                            }
                        } else if lfs_tag_type3(attrs[i].tag) == LFS_TYPE_CREATE as u32 &&
                                unsafe { (*mlist).id } >= lfs_tag_id(attrs[i].tag) {
                            unsafe { (*mlist).id += 1; }
                            if unsafe { (*mlist).type_ } == LFS_TYPE_DIR as u8 {
                                let dir_ptr = mlist as *mut LfsDir;
                                unsafe { (*dir_ptr).pos += 1; }
                            }
                        }
                    }
                }

                while unsafe { (*mlist).id >= (*mlist).m.count && (*mlist).m.split } {
                    // we split and id is on tail now
                    if lfs_pair_cmp(unsafe { (*mlist).m.tail }, lfs.root) != 0 {
                        unsafe { (*mlist).id -= (*mlist).m.count; }
                    }
                    let err = lfs_dir_fetch(lfs, unsafe { &mut (*mlist).m }, unsafe { (*mlist).m.tail });
                    if err != 0 {
                        return err;
                    }
                }
            }
            mlist = unsafe { (*mlist).next };
        }

        return state;
    }
}

fn lfs_dir_orphaningcommit(
    lfs: &mut Lfs,
    dir: &mut LfsMdir,
    attrs: &[LfsMattr],
    attrcount: i32,
) -> i32 {
    // check for any inline files that aren't RAM backed and
    // forcefully evict them, needed for filesystem consistency
    let mut f = lfs.mlist as *mut LfsFile;
    while !f.is_null() {
        let file = unsafe { &mut *f };
        if &file.m as *const _ != dir as *const _ && 
           lfs_pair_cmp(file.m.pair, dir.pair) == 0 &&
           file.type_ == LfsType::Reg as u8 && 
           (file.flags & LfsOpenFlags::Inline as u32 != 0) &&
           file.ctz.size > unsafe { (*lfs.cfg).cache_size } {
            
            let err = lfs_file_outline(lfs, f);
            if err < 0 {
                return err;
            }

            let err = lfs_file_flush(lfs, f);
            if err < 0 {
                return err;
            }
        }
        f = file.next as *mut LfsFile;
    }

    let lpair = [dir.pair[0], dir.pair[1]];
    let mut ldir = dir.clone();
    let mut pdir = LfsMdir::default();
    let state = lfs_dir_relocatingcommit(lfs, &mut ldir, dir.pair, attrs, attrcount, &mut pdir);
    if state < 0 {
        return state;
    }

    // update if we're not in mlist, note we may have already been
    // updated if we are in mlist
    if lfs_pair_cmp(dir.pair, lpair) == 0 {
        *dir = ldir;
    }

    // commit was successful, but may require other changes in the
    // filesystem, these would normally be tail recursive, but we have
    // flattened them here avoid unbounded stack usage

    // need to drop?
    if state == LfsOk::Dropped as i32 {
        // steal state
        let err = lfs_dir_getgstate(lfs, dir, &mut lfs.gdelta);
        if err < 0 {
            return err;
        }

        // steal tail, note that this can't create a recursive drop
        let mut lpair = [pdir.pair[0], pdir.pair[1]];
        lfs_pair_tole32(dir.tail);
        let state = lfs_dir_relocatingcommit(
            lfs,
            &mut pdir,
            lpair,
            &[LfsMattr {
                tag: lfs_tag(LfsType::Tail as u16 + dir.split as u16, 0x3ff, 8),
                buffer: dir.tail.as_ptr() as *const c_void,
            }],
            1,
            core::ptr::null_mut(),
        );
        lfs_pair_fromle32(dir.tail);
        if state < 0 {
            return state;
        }

        ldir = pdir;
    }

    // need to relocate?
    let mut orphans = false;
    let mut state = state;
    let mut lpair = lpair;
    
    while state == LfsOk::Relocated as i32 {
        // Debug log - translated to Rust comment
        // LFS_DEBUG("Relocating {0x%"PRIx32", 0x%"PRIx32"} -> {0x%"PRIx32", 0x%"PRIx32"}",
        //           lpair[0], lpair[1], ldir.pair[0], ldir.pair[1]);
        state = 0;

        // update internal root
        if lfs_pair_cmp(lpair, lfs.root) == 0 {
            lfs.root[0] = ldir.pair[0];
            lfs.root[1] = ldir.pair[1];
        }

        // update internally tracked dirs
        let mut d = lfs.mlist;
        while !d.is_null() {
            let mlist = unsafe { &mut *d };
            if lfs_pair_cmp(lpair, mlist.m.pair) == 0 {
                mlist.m.pair[0] = ldir.pair[0];
                mlist.m.pair[1] = ldir.pair[1];
            }

            if mlist.type_ == LfsType::Dir as u8 {
                let dir_ptr = d as *mut LfsDir;
                let dir = unsafe { &mut *dir_ptr };
                if lfs_pair_cmp(lpair, dir.head) == 0 {
                    dir.head[0] = ldir.pair[0];
                    dir.head[1] = ldir.pair[1];
                }
            }
            
            d = mlist.next;
        }

        // find parent
        let mut pdir = LfsMdir::default();
        let tag = lfs_fs_parent(lfs, &lpair, &mut pdir);
        if tag < 0 && tag != LfsError::NoEnt as i32 {
            return tag;
        }

        let hasparent = tag != LfsError::NoEnt as i32;
        if tag != LfsError::NoEnt as i32 {
            // note that if we have a parent, we must have a pred, so this will
            // always create an orphan
            let err = lfs_fs_preporphans(lfs, 1);
            if err < 0 {
                return err;
            }

            // fix pending move in this pair? this looks like an optimization but
            // is in fact _required_ since relocating may outdate the move.
            let mut moveid = 0x3ff;
            if lfs_gstate_hasmovehere(&lfs.gstate, pdir.pair) {
                moveid = lfs_tag_id(lfs.gstate.tag);
                // Debug log - translated to Rust comment
                // LFS_DEBUG("Fixing move while relocating "
                //         "{0x%"PRIx32", 0x%"PRIx32"} 0x%"PRIx16"\n",
                //         pdir.pair[0], pdir.pair[1], moveid);
                lfs_fs_prepmove(lfs, 0x3ff, core::ptr::null());
                if moveid < lfs_tag_id(tag as u32) {
                    tag -= lfs_tag(0, 1, 0) as i32;
                }
            }

            let ppair = [pdir.pair[0], pdir.pair[1]];
            lfs_pair_tole32(ldir.pair);
            
            let mut attrs = [
                LfsMattr {
                    tag: if moveid != 0x3ff {
                        lfs_tag(LfsType::Delete as u16, moveid, 0)
                    } else {
                        0
                    },
                    buffer: core::ptr::null(),
                },
                LfsMattr {
                    tag: tag as u32,
                    buffer: ldir.pair.as_ptr() as *const c_void,
                },
            ];
            
            state = lfs_dir_relocatingcommit(
                lfs,
                &mut pdir,
                ppair,
                &attrs,
                if moveid != 0x3ff { 2 } else { 1 },
                core::ptr::null_mut(),
            );
            
            lfs_pair_fromle32(ldir.pair);
            if state < 0 {
                return state;
            }

            if state == LfsOk::Relocated as i32 {
                lpair = ppair;
                ldir = pdir;
                orphans = true;
                continue;
            }
        }

        // find pred
        let err = lfs_fs_pred(lfs, &lpair, &mut pdir);
        if err < 0 && err != LfsError::NoEnt as i32 {
            return err;
        }
        debug_assert!(!(hasparent && err == LfsError::NoEnt as i32));

        // if we can't find dir, it must be new
        if err != LfsError::NoEnt as i32 {
            if lfs_gstate_hasorphans(&lfs.gstate) {
                // next step, clean up orphans
                let err = lfs_fs_preporphans(lfs, -if hasparent { 1 } else { 0 });
                if err < 0 {
                    return err;
                }
            }

            // fix pending move in this pair? this looks like an optimization
            // but is in fact _required_ since relocating may outdate the move.
            let mut moveid = 0x3ff;
            if lfs_gstate_hasmovehere(&lfs.gstate, pdir.pair) {
                moveid = lfs_tag_id(lfs.gstate.tag);
                // Debug log - translated to Rust comment
                // LFS_DEBUG("Fixing move while relocating "
                //         "{0x%"PRIx32", 0x%"PRIx32"} 0x%"PRIx16"\n",
                //         pdir.pair[0], pdir.pair[1], moveid);
                lfs_fs_prepmove(lfs, 0x3ff, core::ptr::null());
            }

            // replace bad pair, either we clean up desync, or no desync occured
            lpair = [pdir.pair[0], pdir.pair[1]];
            lfs_pair_tole32(ldir.pair);
            
            let mut attrs = [
                LfsMattr {
                    tag: if moveid != 0x3ff {
                        lfs_tag(LfsType::Delete as u16, moveid, 0)
                    } else {
                        0
                    },
                    buffer: core::ptr::null(),
                },
                LfsMattr {
                    tag: lfs_tag(LfsType::Tail as u16 + pdir.split as u16, 0x3ff, 8),
                    buffer: ldir.pair.as_ptr() as *const c_void,
                },
            ];
            
            state = lfs_dir_relocatingcommit(
                lfs,
                &mut pdir,
                lpair,
                &attrs,
                if moveid != 0x3ff { 2 } else { 1 },
                core::ptr::null_mut(),
            );
            
            lfs_pair_fromle32(ldir.pair);
            if state < 0 {
                return state;
            }

            ldir = pdir;
        }
    }

    if orphans {
        LfsOk::Orphaned as i32
    } else {
        0
    }
}

/// 提交目录修改
/// 
/// 此函数首先通过orphaning commit处理目录修改，然后清理任何可能存在的孤立(orphaned)块
///
/// # Arguments
/// * `lfs` - 文件系统实例
/// * `dir` - 目录结构
/// * `attrs` - 属性数组
/// * `attrcount` - 属性数量
///
/// # Returns
/// * `0` 成功
/// * 负数表示错误代码
fn lfs_dir_commit(lfs: &mut Lfs, dir: &mut LfsMdir, 
                 attrs: *const LfsMattr, attrcount: i32) -> i32 {
    // 进行orphaning commit操作
    let orphans = lfs_dir_orphaningcommit(lfs, dir, attrs, attrcount);
    if orphans < 0 {
        return orphans;
    }

    if orphans > 0 {
        // 确保我们已经移除了所有孤立块，如果没有孤立块这是个空操作，
        // 但如果我们有嵌套块失败，我们可能已经创建了一些孤立块
        let err = lfs_fs_deorphan(lfs, false);
        if err != 0 {
            return err;
        }
    }

    0
}

static fn lfs_mkdir_(lfs: &mut Lfs, path: &[u8]) -> i32 {
    // deorphan if we haven't yet, needed at most once after poweron
    let err = lfs_fs_forceconsistency(lfs);
    if err != 0 {
        return err;
    }

    let mut cwd = LfsMlist {
        next: lfs.mlist,
        id: 0,
        type_: 0,
        m: unsafe { core::mem::zeroed() },
    };
    
    let mut id: u16 = 0;
    let err = lfs_dir_find(lfs, &mut cwd.m, &path, &mut id);
    if !(err == LfsError::NoEnt as i32 && lfs_path_islast(path)) {
        return if err < 0 { err } else { LfsError::Exist as i32 };
    }

    // check that name fits
    let nlen = lfs_path_namelen(path);
    if nlen > lfs.name_max as usize {
        return LfsError::NameTooLong as i32;
    }

    // build up new directory
    lfs_alloc_ckpoint(lfs);
    let mut dir = unsafe { core::mem::zeroed::<LfsMdir>() };
    let err = lfs_dir_alloc(lfs, &mut dir);
    if err != 0 {
        return err;
    }

    // find end of list
    let mut pred = cwd.m.clone();
    while pred.split {
        let err = lfs_dir_fetch(lfs, &mut pred, pred.tail);
        if err != 0 {
            return err;
        }
    }

    // setup dir
    lfs_pair_tole32(pred.tail.as_mut_ptr());
    let err = lfs_dir_commit(lfs, &mut dir, LFS_MKATTRS!(
        {LFS_MKTAG(LfsType::SoftTail as u16, 0x3ff, 8), pred.tail.as_ptr()}
    ));
    lfs_pair_fromle32(pred.tail.as_mut_ptr());
    if err != 0 {
        return err;
    }

    // current block not end of list?
    if cwd.m.split {
        // update tails, this creates a desync
        let err = lfs_fs_preporphans(lfs, 1);
        if err != 0 {
            return err;
        }

        // it's possible our predecessor has to be relocated, and if
        // our parent is our predecessor's predecessor, this could have
        // caused our parent to go out of date, fortunately we can hook
        // ourselves into littlefs to catch this
        cwd.type_ = 0;
        cwd.id = 0;
        lfs.mlist = &mut cwd;

        lfs_pair_tole32(dir.pair.as_mut_ptr());
        let err = lfs_dir_commit(lfs, &mut pred, LFS_MKATTRS!(
            {LFS_MKTAG(LfsType::SoftTail as u16, 0x3ff, 8), dir.pair.as_ptr()}
        ));
        lfs_pair_fromle32(dir.pair.as_mut_ptr());
        if err != 0 {
            lfs.mlist = cwd.next;
            return err;
        }

        lfs.mlist = cwd.next;
        let err = lfs_fs_preporphans(lfs, -1);
        if err != 0 {
            return err;
        }
    }

    // now insert into our parent block
    lfs_pair_tole32(dir.pair.as_mut_ptr());
    let err = lfs_dir_commit(lfs, &mut cwd.m, LFS_MKATTRS!(
        {LFS_MKTAG(LfsType::Create as u16, id, 0), core::ptr::null()},
        {LFS_MKTAG(LfsType::Dir as u16, id, nlen), path.as_ptr()},
        {LFS_MKTAG(LfsType::DirStruct as u16, id, 8), dir.pair.as_ptr()},
        {LFS_MKTAG_IF!(!cwd.m.split, LfsType::SoftTail as u16, 0x3ff, 8), dir.pair.as_ptr()}
    ));
    lfs_pair_fromle32(dir.pair.as_mut_ptr());
    if err != 0 {
        return err;
    }

    0
}

/// Opens a directory
/// 
/// Note: this function has a different signature from the C version due to Rust's
/// return value handling.
fn lfs_dir_open_<'a>(lfs: &mut Lfs, dir: &'a mut LfsDir, path: &str) -> i32 {
    // Find the directory
    let tag = lfs_dir_find(lfs, &mut dir.m, &path, None);
    if tag < 0 {
        return tag;
    }

    // Check if it's a directory
    if lfs_tag_type3(tag) != LfsType::Dir as u16 {
        return LfsError::NotDir as i32;
    }

    // Get the directory pair
    let mut pair = [0u32, 0u32];
    if lfs_tag_id(tag) == 0x3ff {
        // Handle root dir separately
        pair[0] = lfs.root[0];
        pair[1] = lfs.root[1];
    } else {
        // Get dir pair from parent
        let res = lfs_dir_get(
            lfs,
            &mut dir.m,
            LFS_MKTAG(0x700, 0x3ff, 0),
            LFS_MKTAG(LfsType::Struct as u16, lfs_tag_id(tag), 8),
            &mut pair as *mut _ as *mut c_void
        );
        if res < 0 {
            return res;
        }
        lfs_pair_fromle32(&mut pair);
    }

    // Fetch first pair
    let err = lfs_dir_fetch(lfs, &mut dir.m, &pair);
    if err != 0 {
        return err;
    }

    // Setup entry
    dir.head[0] = dir.m.pair[0];
    dir.head[1] = dir.m.pair[1];
    dir.id = 0;
    dir.pos = 0;

    // Add to list of mdirs
    dir.type_ = LfsType::Dir as u8;
    lfs_mlist_append(lfs, dir as *mut LfsDir as *mut LfsMlist);

    0
}

/// 关闭目录并从挂载的目录列表中移除
pub fn lfs_dir_close_(lfs: *mut Lfs, dir: *mut LfsDir) -> i32 {
    // 从mdir列表中移除
    unsafe {
        lfs_mlist_remove(lfs, dir as *mut LfsMlist);
    }

    0
}

fn lfs_dir_read_(lfs: &mut Lfs, dir: &mut LfsDir, info: &mut LfsInfo) -> i32 {
    // 清空info结构体
    *info = LfsInfo {
        type_: 0,
        size: 0,
        name: [0; LFS_NAME_MAX+1],
    };

    // '.' 和 '..' 的特殊偏移
    if dir.pos == 0 {
        info.type_ = LfsType::Dir as u8;
        // 复制 "." 到 info.name
        info.name[0] = b'.';
        info.name[1] = 0;
        dir.pos += 1;
        return 1; // true
    } else if dir.pos == 1 {
        info.type_ = LfsType::Dir as u8;
        // 复制 ".." 到 info.name
        info.name[0] = b'.';
        info.name[1] = b'.';
        info.name[2] = 0;
        dir.pos += 1;
        return 1; // true
    }

    loop {
        if dir.id == dir.m.count {
            if !dir.m.split {
                return 0; // false
            }

            match lfs_dir_fetch(lfs, &mut dir.m, dir.m.tail) {
                err if err < 0 => return err,
                _ => {}
            }

            dir.id = 0;
        }

        match lfs_dir_getinfo(lfs, &dir.m, dir.id, info) {
            err if err < 0 && err != LfsError::NoEnt as i32 => return err,
            err if err != LfsError::NoEnt as i32 => {
                dir.id += 1;
                dir.pos += 1;
                return 1; // true
            },
            _ => dir.id += 1,
        }
    }
}

/// 将目录指针移动到指定偏移位置
pub fn lfs_dir_seek_(lfs: &mut Lfs, dir: &mut LfsDir, off: LfsOff) -> i32 {
    // 简单地从头部目录开始遍历
    let err = lfs_dir_rewind_(lfs, dir);
    if err != 0 {
        return err;
    }

    // 前两个项是 ./ 和 ..
    dir.pos = lfs_util::min(2, off);
    let mut remaining_off = off - dir.pos;

    // 跳过超级块条目
    dir.id = if remaining_off > 0 && lfs_pair_cmp(dir.head, lfs.root) == 0 { 1 } else { 0 };

    while remaining_off > 0 {
        if dir.id == dir.m.count {
            if !dir.m.split {
                return LfsError::Inval as i32;
            }

            let err = lfs_dir_fetch(lfs, &mut dir.m, dir.m.tail);
            if err != 0 {
                return err;
            }

            dir.id = 0;
        }

        let diff = lfs_util::min((dir.m.count - dir.id) as LfsOff, remaining_off);
        dir.id += diff as u16;
        dir.pos += diff;
        remaining_off -= diff;
    }

    0
}

/// 获取目录的当前位置
///
/// # Arguments
///
/// * `lfs` - 文件系统指针
/// * `dir` - 目录指针
///
/// # Returns
///
/// 当前目录位置
pub fn lfs_dir_tell_(lfs: *mut Lfs, dir: *mut LfsDir) -> LfsSoff {
    // 忽略lfs参数
    unsafe {
        (*dir).pos
    }
}

/// 重置目录状态到起始位置
fn lfs_dir_rewind_(lfs: &mut Lfs, dir: &mut LfsDir) -> i32 {
    // 重载头部目录
    let err = lfs_dir_fetch(lfs, &mut dir.m, dir.head);
    if err != 0 {
        return err;
    }

    dir.id = 0;
    dir.pos = 0;
    0
}

/// 计算CTZ (count trailing zeros) 索引
fn lfs_ctz_index(lfs: &mut Lfs, off: &mut LfsOff) -> LfsBlock {
    let size = *off;
    let b = (*lfs.cfg).block_size - 2 * 4;
    let mut i = size / b;
    if i == 0 {
        return 0;
    }

    i = (size - 4 * (lfs_util::lfs_popc(i - 1) + 2)) / b;
    *off = size - b * i - 4 * lfs_util::lfs_popc(i);
    return i;
}

/// 查找给定CTZ跳跃列表中的位置
pub fn lfs_ctz_find(
    lfs: &mut Lfs,
    pcache: &LfsCache,
    rcache: &mut LfsCache,
    head: LfsBlock,
    size: LfsSize,
    pos: LfsSize,
    block: &mut LfsBlock,
    off: &mut LfsOff,
) -> i32 {
    if size == 0 {
        *block = LFS_BLOCK_NULL;
        *off = 0;
        return 0;
    }

    let mut current = lfs_util::lfs_ctz_index(lfs, &(size - 1));
    let target = lfs_util::lfs_ctz_index(lfs, &pos);
    let mut head_value = head;

    while current > target {
        let skip = lfs_util::lfs_min(
            lfs_util::lfs_npw2(current - target + 1) - 1,
            lfs_util::lfs_ctz(current),
        );

        let err = lfs_util::lfs_bd_read(
            lfs,
            pcache,
            rcache,
            core::mem::size_of::<LfsBlock>() as LfsSize,
            head_value,
            4 * skip,
            &mut head_value as *mut _ as *mut core::ffi::c_void,
            core::mem::size_of::<LfsBlock>() as LfsSize,
        );
        
        head_value = lfs_util::lfs_fromle32(head_value);
        if err != 0 {
            return err;
        }

        current -= 1 << skip;
    }

    *block = head_value;
    *off = pos;
    return 0;
}

/// 扩展计数尾随零结构
/// 
/// 这个函数处理LFS中块的分配并扩展CTZ结构
fn lfs_ctz_extend(
    lfs: &mut Lfs,
    pcache: &mut LfsCache,
    rcache: &mut LfsCache,
    head: LfsBlock,
    size: LfsSize,
    block: &mut LfsBlock,
    off: &mut LfsOff,
) -> i32 {
    loop {
        // 获取一个新块
        let mut nblock = 0;
        let err = lfs_alloc(lfs, &mut nblock);
        if err != 0 {
            return err;
        }

        // 尝试擦除该块
        let err = lfs_bd_erase(lfs, nblock);
        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                // 如果块损坏，尝试重新分配
                lfs_debug!("Bad block at 0x{:x}", nblock);
                
                // 清除缓存并尝试新块
                lfs_cache_drop(lfs, pcache);
                continue;
            }
            return err;
        }

        if size == 0 {
            *block = nblock;
            *off = 0;
            return 0;
        }

        let mut noff = size - 1;
        let index = lfs_ctz_index(lfs, &mut noff);
        noff = noff + 1;

        // 如果最后一个块不完整，只需复制它
        if noff != unsafe { (*lfs.cfg).block_size } {
            for i in 0..noff {
                let mut data: u8 = 0;
                let err = lfs_bd_read(
                    lfs,
                    core::ptr::null_mut(),
                    rcache,
                    noff - i,
                    head,
                    i,
                    &mut data as *mut u8 as *mut core::ffi::c_void,
                    1,
                );
                if err != 0 {
                    return err;
                }

                let err = lfs_bd_prog(
                    lfs,
                    pcache,
                    rcache,
                    true,
                    nblock,
                    i,
                    &data as *const u8 as *const core::ffi::c_void,
                    1,
                );
                if err != 0 {
                    if err == LfsError::Corrupt as i32 {
                        // 如果块损坏，尝试重新分配
                        lfs_debug!("Bad block at 0x{:x}", nblock);
                        
                        // 清除缓存并尝试新块
                        lfs_cache_drop(lfs, pcache);
                        continue;
                    }
                    return err;
                }
            }

            *block = nblock;
            *off = noff;
            return 0;
        }

        // 追加块
        let index = index + 1;
        let skips = lfs_ctz(index) + 1;
        let mut nhead = head;
        for i in 0..skips {
            let nhead_le = lfs_tole32(nhead);
            let err = lfs_bd_prog(
                lfs,
                pcache,
                rcache,
                true,
                nblock,
                4 * i,
                &nhead_le as *const u32 as *const core::ffi::c_void,
                4,
            );
            nhead = lfs_fromle32(nhead_le);
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    // 如果块损坏，尝试重新分配
                    lfs_debug!("Bad block at 0x{:x}", nblock);
                    
                    // 清除缓存并尝试新块
                    lfs_cache_drop(lfs, pcache);
                    continue;
                }
                return err;
            }

            if i != skips - 1 {
                let mut nhead_le = 0u32;
                let err = lfs_bd_read(
                    lfs,
                    core::ptr::null_mut(),
                    rcache,
                    core::mem::size_of::<u32>() as LfsSize,
                    nhead,
                    4 * i,
                    &mut nhead_le as *mut u32 as *mut core::ffi::c_void,
                    core::mem::size_of::<u32>() as LfsSize,
                );
                nhead = lfs_fromle32(nhead_le);
                if err != 0 {
                    return err;
                }
            }
        }

        *block = nblock;
        *off = 4 * skips;
        return 0;
    }
}

/// 打开文件配置
fn lfs_file_opencfg_(lfs: &mut Lfs, file: &mut LfsFile, 
        path: &str, flags: u32, cfg: &LfsFileConfig) -> i32 {
    #[cfg(not(feature = "LFS_READONLY"))]
    {
        // 如果还没清理孤立节点，则需要清理，上电后最多需要一次
        if (flags & LfsOpenFlags::WrOnly as u32) == LfsOpenFlags::WrOnly as u32 {
            let err = lfs_fs_forceconsistency(lfs);
            if err != 0 {
                return err;
            }
        }
    }
    #[cfg(feature = "LFS_READONLY")]
    {
        lfs_util::LFS_ASSERT((flags & LfsOpenFlags::RdOnly as u32) == LfsOpenFlags::RdOnly as u32);
    }

    // 设置简单文件详情
    file.cfg = cfg;
    file.flags = flags;
    file.pos = 0;
    file.off = 0;
    file.cache.buffer = core::ptr::null_mut();

    // 如果文件条目不存在则分配
    let mut err;
    let tag = lfs_dir_find(lfs, &mut file.m, &path, &mut file.id);
    if tag < 0 && !(tag == LfsError::NoEnt as i32 && lfs_path_islast(path)) {
        err = tag;
        goto cleanup;
    }

    // 获取ID，添加到mdirs列表以捕获更新更改
    file.type_ = LfsType::Reg as u8;
    lfs_mlist_append(lfs, file as *mut LfsFile as *mut LfsMlist);

    #[cfg(feature = "LFS_READONLY")]
    if tag == LfsError::NoEnt as i32 {
        err = LfsError::NoEnt as i32;
        goto cleanup;
    }
    #[cfg(not(feature = "LFS_READONLY"))]
    if tag == LfsError::NoEnt as i32 {
        if (flags & LfsOpenFlags::Creat as u32) == 0 {
            err = LfsError::NoEnt as i32;
            goto cleanup;
        }

        // 不允许尾部斜杠
        if lfs_path_isdir(path) {
            err = LfsError::NotDir as i32;
            goto cleanup;
        }

        // 检查名称是否合适
        let nlen = lfs_path_namelen(path);
        if nlen > lfs.name_max {
            err = LfsError::NameTooLong as i32;
            goto cleanup;
        }

        // 获取下一个插槽并创建条目以记住名称
        err = lfs_dir_commit(lfs, &mut file.m, &[
            LfsMattr {
                tag: LFS_MKTAG(LfsType::Create as u32, file.id, 0), 
                buffer: core::ptr::null()
            },
            LfsMattr {
                tag: LFS_MKTAG(LfsType::Reg as u32, file.id, nlen as u32), 
                buffer: path.as_ptr() as *const c_void
            },
            LfsMattr {
                tag: LFS_MKTAG(LfsType::InlineStruct as u32, file.id, 0), 
                buffer: core::ptr::null()
            }
        ]);

        // 有可能文件名不适合元数据块，例如256字节的文件名
        // 不会适合128字节的块。
        err = if err == LfsError::NoSpc as i32 { 
            LfsError::NameTooLong as i32 
        } else { 
            err 
        };
        if err != 0 {
            goto cleanup;
        }

        tag = LFS_MKTAG(LfsType::InlineStruct as u32, 0, 0);
    } else if (flags & LfsOpenFlags::Excl as u32) != 0 {
        err = LfsError::Exist as i32;
        goto cleanup;
    } else if lfs_tag_type3(tag) != LfsType::Reg as u32 {
        err = LfsError::IsDir as i32;
        goto cleanup;
    #[cfg(not(feature = "LFS_READONLY"))]
    } else if (flags & LfsOpenFlags::Trunc as u32) != 0 {
        // 如果请求则截断
        tag = LFS_MKTAG(LfsType::InlineStruct as u32, file.id, 0);
        file.flags |= LfsOpenFlags::Dirty as u32;
    } else {
        // 尝试加载磁盘上的内容，如果是内联的稍后会修复
        tag = lfs_dir_get(lfs, &mut file.m, LFS_MKTAG(0x700, 0x3ff, 0),
                LFS_MKTAG(LfsType::Struct as u32, file.id, 8), &mut file.ctz);
        if tag < 0 {
            err = tag;
            goto cleanup;
        }
        lfs_ctz_fromle32(&mut file.ctz);
    }

    // 获取属性
    for i in 0..unsafe { (*file.cfg).attr_count } {
        // 如果以只读/读写模式打开
        if (file.flags & LfsOpenFlags::RdOnly as u32) == LfsOpenFlags::RdOnly as u32 {
            let attrs = unsafe { (*file.cfg).attrs.add(i as usize) };
            let res = lfs_dir_get(lfs, &mut file.m,
                    LFS_MKTAG(0x7ff, 0x3ff, 0),
                    LFS_MKTAG(LfsType::UserAttr as u32 + unsafe { (*attrs).type_ } as u32,
                        file.id, unsafe { (*attrs).size }), 
                    unsafe { (*attrs).buffer });
            if res < 0 && res != LfsError::NoEnt as i32 {
                err = res;
                goto cleanup;
            }
        }

        #[cfg(not(feature = "LFS_READONLY"))]
        // 如果以写入/读写模式打开
        if (file.flags & LfsOpenFlags::WrOnly as u32) == LfsOpenFlags::WrOnly as u32 {
            let attrs = unsafe { (*file.cfg).attrs.add(i as usize) };
            if unsafe { (*attrs).size } > lfs.attr_max {
                err = LfsError::NoSpc as i32;
                goto cleanup;
            }

            file.flags |= LfsOpenFlags::Dirty as u32;
        }
    }

    // 如果需要则分配缓冲区
    if !unsafe { (*file.cfg).buffer }.is_null() {
        file.cache.buffer = unsafe { (*file.cfg).buffer } as *mut u8;
    } else {
        file.cache.buffer = lfs_malloc(lfs.cfg.block_size) as *mut u8;
        if file.cache.buffer.is_null() {
            err = LfsError::NoMem as i32;
            goto cleanup;
        }
    }

    // 清零以避免信息泄漏
    lfs_cache_zero(lfs, &mut file.cache);

    if lfs_tag_type3(tag) == LfsType::InlineStruct as u32 {
        // 加载内联文件
        file.ctz.head = LFS_BLOCK_INLINE;
        file.ctz.size = lfs_tag_size(tag);
        file.flags |= LfsOpenFlags::Inline as u32;
        file.cache.block = file.ctz.head;
        file.cache.off = 0;
        file.cache.size = unsafe { (*lfs.cfg).cache_size };

        // 不总是读取（可能是新/截断文件）
        if file.ctz.size > 0 {
            let res = lfs_dir_get(lfs, &mut file.m,
                    LFS_MKTAG(0x700, 0x3ff, 0),
                    LFS_MKTAG(LfsType::Struct as u32, file.id,
                        lfs_util::lfs_min(file.cache.size, 0x3fe)),
                    file.cache.buffer as *mut c_void);
            if res < 0 {
                err = res;
                goto cleanup;
            }
        }
    }

    return 0;

cleanup:
    // 清理残留资源
    #[cfg(not(feature = "LFS_READONLY"))]
    {
        file.flags |= LfsOpenFlags::Erred as u32;
    }
    lfs_file_close_(lfs, file);
    return err;
}

#[cfg(not(feature = "LFS_NO_MALLOC"))]
fn lfs_file_open_(lfs: &mut Lfs, file: &mut LfsFile,
        path: &str, flags: u32) -> i32 {
    static DEFAULTS: LfsFileConfig = LfsFileConfig {
        buffer: core::ptr::null_mut(),
        attrs: core::ptr::null_mut(),
        attr_count: 0,
    };
    
    let err = lfs_file_opencfg_(lfs, file, path, flags, &DEFAULTS);
    return err;
}

fn lfs_file_close_(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    #[cfg(not(feature = "LFS_READONLY"))]
    let err = lfs_file_sync_(lfs, file);
    #[cfg(feature = "LFS_READONLY")]
    let err = 0;

    // 从mdirs列表中移除
    lfs_mlist_remove(lfs, file as *mut LfsFile as *mut LfsMlist);

    // 清理内存
    if unsafe { (*file.cfg).buffer }.is_null() {
        lfs_free(file.cache.buffer as *mut c_void);
    }

    return err;
}

#[cfg(not(feature = "LFS_READONLY"))]
fn lfs_file_relocate(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    loop {
        // 只是将现有内容重新定位到新块中
        let mut nblock: LfsBlock = 0;
        let err = lfs_alloc(lfs, &mut nblock);
        if err != 0 {
            return err;
        }

        err = lfs_bd_erase(lfs, nblock);
        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                goto relocate;
            }
            return err;
        }

        // 要么从脏缓存读取，要么从磁盘读取
        for i in 0..file.off {
            let mut data: u8 = 0;
            if (file.flags & LfsOpenFlags::Inline as u32) != 0 {
                err = lfs_dir_getread(lfs, &mut file.m,
                        core::ptr::null_mut(), &mut file.cache, file.off - i,
                        LFS_MKTAG(0xfff, 0x1ff, 0),
                        LFS_MKTAG(LfsType::InlineStruct as u32, file.id, 0),
                        i, &mut data as *mut u8 as *mut c_void, 1);
                if err != 0 {
                    return err;
                }
            } else {
                err = lfs_bd_read(lfs,
                        &mut file.cache, &mut lfs.rcache, file.off - i,
                        file.block, i, &mut data as *mut u8 as *mut c_void, 1);
                if err != 0 {
                    return err;
                }
            }

            err = lfs_bd_prog(lfs,
                    &mut lfs.pcache, &mut lfs.rcache, true,
                    nblock, i, &data as *const u8 as *const c_void, 1);
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    goto relocate;
                }
                return err;
            }
        }

        // 复制文件的新状态
        unsafe {
            core::ptr::copy_nonoverlapping(
                lfs.pcache.buffer, 
                file.cache.buffer, 
                (*lfs.cfg).cache_size as usize
            );
        }
        file.cache.block = lfs.pcache.block;
        file.cache.off = lfs.pcache.off;
        file.cache.size = lfs.pcache.size;
        lfs_cache_zero(lfs, &mut lfs.pcache);

        file.block = nblock;
        file.flags |= LfsOpenFlags::Writing as u32;
        return 0;

relocate:
        lfs_util::LFS_DEBUG("Bad block at 0x{:x}", nblock);

        // 只需清除缓存并尝试新块
        lfs_cache_drop(lfs, &mut lfs.pcache);
    }
}

#[cfg(not(feature = "LFS_READONLY"))]
fn lfs_file_outline(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    file.off = file.pos;
    lfs_alloc_ckpoint(lfs);
    let err = lfs_file_relocate(lfs, file);
    if err != 0 {
        return err;
    }

    file.flags &= !LfsOpenFlags::Inline as u32;
    return 0;
}

fn lfs_file_flush(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    if (file.flags & LfsOpenFlags::Reading as u32) != 0 {
        if (file.flags & LfsOpenFlags::Inline as u32) == 0 {
            lfs_cache_drop(lfs, &mut file.cache);
        }
        file.flags &= !LfsOpenFlags::Reading as u32;
    }

    #[cfg(not(feature = "LFS_READONLY"))]
    if (file.flags & LfsOpenFlags::Writing as u32) != 0 {
        let pos = file.pos;

        if (file.flags & LfsOpenFlags::Inline as u32) == 0 {
            // 复制当前分支之后的任何内容
            let mut orig = LfsFile {
                ctz: LfsCtz {
                    head: file.ctz.head,
                    size: file.ctz.size,
                },
                flags: LfsOpenFlags::RdOnly as u32,
                pos: file.pos,
                cache: lfs.rcache.clone(),
                ..Default::default()
            };
            lfs_cache_drop(lfs, &mut lfs.rcache);

            while file.pos < file.ctz.size {
                // 一次复制一个字节，依靠缓存使其高效
                let mut data: u8 = 0;
                let res = lfs_file_flushedread(lfs, &mut orig, &mut data as *mut u8 as *

/// Opens a file with default configuration
///
/// This function wraps lfs_file_opencfg_ with default configuration settings
fn lfs_file_open_(lfs: &mut Lfs, file: &mut LfsFile, path: &str, flags: u32) -> i32 {
    // Create default file configuration with all fields zeroed
    let defaults = LfsFileConfig {
        buffer: core::ptr::null_mut(),
        attrs: core::ptr::null_mut(),
        attr_count: 0,
    };
    
    // Call the opencfg function with default configuration
    lfs_file_opencfg_(lfs, file, path, flags, &defaults)
}

fn lfs_file_close_(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    #[cfg(not(feature = "LFS_READONLY"))]
    let err = lfs_file_sync_(lfs, file);
    #[cfg(feature = "LFS_READONLY")]
    let err = 0;

    // remove from list of mdirs
    lfs_mlist_remove(lfs, file as *mut LfsFile as *mut LfsMlist);

    // clean up memory
    if (file.cfg as *const LfsFileConfig).is_null() || unsafe { (*file.cfg).buffer.is_null() } {
        lfs_free(file.cache.buffer as *mut c_void);
    }

    err
}

/// 重定位文件到新块
fn lfs_file_relocate(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    loop {
        // 仅将现有内容重定位到新块
        let mut nblock: LfsBlock = 0;
        let err = lfs_alloc(lfs, &mut nblock);
        if err != 0 {
            return err;
        }

        let err = lfs_bd_erase(lfs, nblock);
        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                continue; // goto relocate
            }
            return err;
        }

        // 从脏缓存或磁盘读取
        for i in 0..file.off {
            let mut data: u8 = 0;
            if (file.flags & LfsOpenFlags::Inline as u32) != 0 {
                let err = lfs_dir_getread(
                    lfs,
                    &file.m,
                    // 注意我们会在内联文件变脏之前将其驱逐
                    core::ptr::null(),
                    &file.cache,
                    file.off - i,
                    lfs_util::lfs_tag_mktag(0xfff, 0x1ff, 0),
                    lfs_util::lfs_tag_mktag(LfsType::InlineStruct as u16, file.id, 0),
                    i,
                    &mut data as *mut u8 as *mut c_void,
                    1,
                );
                if err != 0 {
                    return err;
                }
            } else {
                let err = lfs_bd_read(
                    lfs,
                    &file.cache,
                    &lfs.rcache,
                    file.off - i,
                    file.block,
                    i,
                    &mut data as *mut u8 as *mut c_void,
                    1,
                );
                if err != 0 {
                    return err;
                }
            }

            let err = lfs_bd_prog(
                lfs,
                &lfs.pcache,
                &lfs.rcache,
                true,
                nblock,
                i,
                &data as *const u8 as *const c_void,
                1,
            );
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    break; // goto relocate
                }
                return err;
            }
        }

        // 复制文件的新状态
        // 安全地复制缓存内容
        unsafe {
            core::ptr::copy_nonoverlapping(
                lfs.pcache.buffer,
                file.cache.buffer,
                lfs.cfg.read().cache_size as usize,
            );
        }
        file.cache.block = lfs.pcache.block;
        file.cache.off = lfs.pcache.off;
        file.cache.size = lfs.pcache.size;
        lfs_cache_zero(lfs, &mut lfs.pcache);

        file.block = nblock;
        file.flags |= LfsOpenFlags::Writing as u32;
        return 0;
    }
}

/// 将内联文件转换为常规外部存储文件
/// 
/// 当内联文件需要增长超过内联容量时调用
pub fn lfs_file_outline(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    file.off = file.pos;
    lfs_alloc_ckpoint(lfs);
    let err = lfs_file_relocate(lfs, file);
    if err != 0 {
        return err;
    }

    file.flags &= !LfsOpenFlags::Inline as u32;
    0
}

/// Flush any pending writes to file
fn lfs_file_flush(lfs: &mut Lfs, file: &mut LfsFile) -> i32 {
    if file.flags & LfsOpenFlags::Reading as u32 != 0 {
        if file.flags & LfsOpenFlags::Inline as u32 == 0 {
            lfs_cache_drop(lfs, &mut file.cache);
        }
        file.flags &= !(LfsOpenFlags::Reading as u32);
    }

    #[cfg(not(feature = "LFS_READONLY"))]
    {
        if file.flags & LfsOpenFlags::Writing as u32 != 0 {
            let pos = file.pos;

            if file.flags & LfsOpenFlags::Inline as u32 == 0 {
                // copy over anything after current branch
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
                        tail: [0, 0] 
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

                while file.pos < file.ctz.size {
                    // copy over a byte at a time, leave it up to caching
                    // to make this efficient
                    let mut data: u8 = 0;
                    let res = lfs_file_flushedread(lfs, &mut orig, &mut data as *mut u8 as *mut c_void, 1);
                    if res < 0 {
                        return res;
                    }

                    let res = lfs_file_flushedwrite(lfs, file, &data as *const u8 as *const c_void, 1);
                    if res < 0 {
                        return res;
                    }

                    // keep our reference to the rcache in sync
                    if lfs.rcache.block != LFS_BLOCK_NULL {
                        lfs_cache_drop(lfs, &mut orig.cache);
                        lfs_cache_drop(lfs, &mut lfs.rcache);
                    }
                }

                // write out what we have
                loop {
                    let err = lfs_bd_flush(lfs, &mut file.cache, &mut lfs.rcache, true);
                    if err != 0 {
                        if err == LfsError::Corrupt as i32 {
                            // handle relocate case
                            debug!("Bad block at 0x{:x}", file.block);
                            let err = lfs_file_relocate(lfs, file);
                            if err != 0 {
                                return err;
                            }
                            continue;
                        }
                        return err;
                    }
                    break;
                }
            } else {
                file.pos = lfs_max(file.pos, file.ctz.size);
            }

            // actual file updates
            file.ctz.head = file.block;
            file.ctz.size = file.pos;
            file.flags &= !(LfsOpenFlags::Writing as u32);
            file.flags |= LfsOpenFlags::Dirty as u32;

            file.pos = pos;
        }
    }

    0
}

/// 同步文件内容与存储
fn lfs_file_sync_(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let flags = unsafe { (*file).flags };
    
    // 如果文件有错误，不安全执行任何操作
    if (flags & LfsOpenFlags::Erred as u32) != 0 {
        return 0;
    }

    // 刷新文件内容
    let mut err = unsafe { lfs_file_flush(lfs, file) };
    if err != 0 {
        unsafe { (*file).flags |= LfsOpenFlags::Erred as u32 };
        return err;
    }

    // 如果文件脏且有有效的元数据对
    unsafe {
        if ((*file).flags & LfsOpenFlags::Dirty as u32) != 0 && 
           !lfs_pair_isnull((*file).m.pair) {
            
            // 在提交元数据前，需要同步磁盘以确保数据写入不会在元数据写入后完成
            if ((*file).flags & LfsOpenFlags::Inline as u32) == 0 {
                err = lfs_bd_sync(lfs, &(*lfs).pcache, &(*lfs).rcache, false);
                if err != 0 {
                    return err;
                }
            }

            // 更新目录条目
            let mut type_: u16;
            let buffer: *const c_void;
            let mut size: LfsSize;
            let mut ctz = LfsCtz { head: 0, size: 0 };

            if ((*file).flags & LfsOpenFlags::Inline as u32) != 0 {
                // 内联整个文件
                type_ = LfsType::InlineStruct as u16;
                buffer = (*file).cache.buffer as *const c_void;
                size = (*file).ctz.size;
            } else {
                // 更新ctz引用
                type_ = LfsType::CtzStruct as u16;
                // 复制ctz，以便在重定位期间alloc正常工作
                ctz = (*file).ctz.clone();
                lfs_ctz_tole32(&mut ctz);
                buffer = &ctz as *const _ as *const c_void;
                size = core::mem::size_of::<LfsCtz>() as LfsSize;
            }

            // 提交文件数据和属性
            err = lfs_dir_commit(lfs, &mut (*file).m, LFS_MKATTRS!(
                (LFS_MKTAG(type_, (*file).id, size), buffer),
                (LFS_MKTAG(LfsType::FromUserAttrs as u16, (*file).id, 
                    (*(*file).cfg).attr_count), (*(*file).cfg).attrs as *const c_void)
            ));

            if err != 0 {
                (*file).flags |= LfsOpenFlags::Erred as u32;
                return err;
            }

            (*file).flags &= !(LfsOpenFlags::Dirty as u32);
        }
    }

    return 0;
}

fn lfs_file_flushedread(lfs: &mut Lfs, file: &mut LfsFile, 
    buffer: *mut c_void, size: LfsSize) -> LfsSsize {
    let mut data = buffer as *mut u8;
    let mut nsize = size;

    if file.pos >= file.ctz.size {
        // eof if past end
        return 0;
    }

    let size = lfs_util::min(size, file.ctz.size - file.pos);
    let mut nsize = size;

    while nsize > 0 {
        // check if we need a new block
        if (file.flags & LfsOpenFlags::Reading as u32 == 0) ||
           file.off == unsafe { (*file.cfg).block_size } {
            if (file.flags & LfsOpenFlags::Inline as u32) == 0 {
                match lfs_ctz_find(lfs, None, &mut file.cache,
                        file.ctz.head, file.ctz.size,
                        file.pos, &mut file.block, &mut file.off) {
                    LfsError::Ok => {},
                    err => return err as LfsSsize,
                }
            } else {
                file.block = LFS_BLOCK_INLINE;
                file.off = file.pos;
            }

            file.flags |= LfsOpenFlags::Reading as u32;
        }

        // read as much as we can in current block
        let diff = lfs_util::min(nsize, unsafe { (*lfs.cfg).block_size } - file.off);
        let result = if (file.flags & LfsOpenFlags::Inline as u32) != 0 {
            lfs_dir_getread(lfs, &mut file.m,
                    None, &mut file.cache, unsafe { (*lfs.cfg).block_size },
                    lfs_util::mktag(0xfff, 0x1ff, 0),
                    lfs_util::mktag(LfsType::InlineStruct as u16, file.id, 0),
                    file.off, data, diff)
        } else {
            lfs_bd_read(lfs,
                    None, &mut file.cache, unsafe { (*lfs.cfg).block_size },
                    file.block, file.off, data, diff)
        };

        match result {
            LfsError::Ok => {},
            err => return err as LfsSsize,
        }

        file.pos += diff;
        file.off += diff;
        data = unsafe { data.add(diff as usize) };
        nsize -= diff;
    }

    return size as LfsSsize;
}

fn lfs_file_read_(lfs: &mut Lfs, file: &mut LfsFile, buffer: *mut c_void, size: LfsSize) -> LfsSsize {
    debug_assert!((file.flags & LfsOpenFlags::RdOnly as u32) == LfsOpenFlags::RdOnly as u32);

    #[cfg(not(feature = "LFS_READONLY"))]
    if (file.flags & LfsOpenFlags::Writing as u32) != 0 {
        // flush out any writes
        let err = lfs_file_flush(lfs, file);
        if err != 0 {
            return err;
        }
    }

    lfs_file_flushedread(lfs, file, buffer, size)
}

/// 写入文件内容并确保数据刷新到存储
fn lfs_file_flushedwrite(lfs: &mut Lfs, file: &mut LfsFile, 
                         buffer: *const c_void, size: LfsSize) -> LfsSsize {
    let data = buffer as *const u8;
    let mut nsize = size;

    if (file.flags & LfsOpenFlags::Inline as u32 != 0) &&
            core::cmp::max(file.pos + nsize, file.ctz.size) > unsafe { (*lfs.cfg).inline_max } {
        // 内联文件不再适合
        let err = lfs_file_outline(lfs, file);
        if err != 0 {
            file.flags |= LfsOpenFlags::Erred as u32;
            return err;
        }
    }

    let mut data_ptr = data;
    while nsize > 0 {
        // 检查是否需要新块
        if (file.flags & LfsOpenFlags::Writing as u32 == 0) ||
                file.off == unsafe { (*lfs.cfg).block_size } {
            if (file.flags & LfsOpenFlags::Inline as u32 == 0) {
                if (file.flags & LfsOpenFlags::Writing as u32 == 0) && file.pos > 0 {
                    // 找出我们要从哪个块扩展
                    let mut dummy: LfsOff = 0;
                    let err = lfs_ctz_find(lfs, core::ptr::null(), &mut file.cache,
                            file.ctz.head, file.ctz.size,
                            file.pos - 1, &mut file.block, &mut dummy);
                    if err != 0 {
                        file.flags |= LfsOpenFlags::Erred as u32;
                        return err;
                    }

                    // 标记缓存为脏，因为我们可能已经读取了数据到其中
                    lfs_cache_zero(lfs, &mut file.cache);
                }

                // 用新块扩展文件
                lfs_alloc_ckpoint(lfs);
                let err = lfs_ctz_extend(lfs, &mut file.cache, &mut lfs.rcache,
                        file.block, file.pos,
                        &mut file.block, &mut file.off);
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

        // 在当前块中编程尽可能多的内容
        let diff = core::cmp::min(nsize, unsafe { (*lfs.cfg).block_size } - file.off);
        'prog_loop: loop {
            let err = lfs_bd_prog(lfs, &mut file.cache, &mut lfs.rcache, true,
                    file.block, file.off, data_ptr as *const c_void, diff);
            if err != 0 {
                if err == LfsError::Corrupt as i32 {
                    // 需要重新定位
                    let err = lfs_file_relocate(lfs, file);
                    if err != 0 {
                        file.flags |= LfsOpenFlags::Erred as u32;
                        return err;
                    }
                    continue 'prog_loop;
                }
                file.flags |= LfsOpenFlags::Erred as u32;
                return err;
            }

            break;
        }

        file.pos += diff;
        file.off += diff;
        unsafe { data_ptr = data_ptr.add(diff as usize); }
        nsize -= diff;

        lfs_alloc_ckpoint(lfs);
    }

    return size as LfsSsize;
}

/// 写入文件内容
///
/// 将缓冲区中的数据写入到文件。
pub unsafe fn lfs_file_write_(
    lfs: *mut Lfs,
    file: *mut LfsFile,
    buffer: *const c_void,
    size: LfsSize,
) -> LfsSsize {
    // 断言文件是以只写模式打开的
    debug_assert!(((*file).flags & LfsOpenFlags::WrOnly as u32) == LfsOpenFlags::WrOnly as u32);

    if (*file).flags & LfsOpenFlags::Reading as u32 != 0 {
        // 如果先前正在读取，需要先清空文件缓冲区
        let err = lfs_file_flush(lfs, file);
        if err < 0 {
            return err;
        }
    }

    // 如果是追加模式且当前位置小于文件大小，则移动到文件末尾
    if ((*file).flags & LfsOpenFlags::Append as u32 != 0) && (*file).pos < (*file).ctz.size {
        (*file).pos = (*file).ctz.size;
    }

    // 检查是否超过文件大小限制
    if (*file).pos + size > (*lfs).file_max {
        return LfsError::FBig as LfsSsize;
    }

    // 如果文件不处于写入状态，且当前位置大于文件大小，则需要用0填充
    if ((*file).flags & LfsOpenFlags::Writing as u32 == 0) && (*file).pos > (*file).ctz.size {
        let pos = (*file).pos;
        (*file).pos = (*file).ctz.size;

        // 用0填充到当前位置
        while (*file).pos < pos {
            let zero: u8 = 0;
            let res = lfs_file_flushedwrite(lfs, file, &zero as *const u8 as *const c_void, 1);
            if res < 0 {
                return res;
            }
        }
    }

    // 执行实际写入操作
    let nsize = lfs_file_flushedwrite(lfs, file, buffer, size);
    if nsize < 0 {
        return nsize;
    }

    // 清除错误标志
    (*file).flags &= !(LfsOpenFlags::Erred as u32);
    nsize
}

/// 在文件中寻找位置
///
/// 根据指定的偏移量和起始位置，计算并设置新的文件位置
fn lfs_file_seek_(lfs: &mut Lfs, file: &mut LfsFile, off: LfsSoff, whence: i32) -> LfsSoff {
    // 找到新位置
    //
    // 幸运的是，littlefs限制为31位文件大小，所以我们
    // 不需要太担心整数溢出问题
    let mut npos = file.pos;
    if whence == LfsWhenceFlags::Set as i32 {
        npos = off as LfsOff;
    } else if whence == LfsWhenceFlags::Cur as i32 {
        npos = file.pos.wrapping_add(off as LfsOff);
    } else if whence == LfsWhenceFlags::End as i32 {
        // 安全地使用unsafe代码调用外部函数
        let file_size = unsafe { lfs_file_size_(lfs, file) };
        npos = file_size.wrapping_add(off as LfsOff);
    }

    // 检查文件位置是否超出范围
    if npos > unsafe { (*lfs.cfg).file_max } {
        // 文件位置超出范围
        return LfsError::Inval as LfsSoff;
    }

    // 如果位置没有变化，直接返回
    if file.pos == npos {
        // 无操作 - 位置未改变
        return npos as LfsSoff;
    }

    // 如果我们只是读取且新偏移量仍在文件缓存中
    // 可以避免刷新并重新读取数据
    if (file.flags & LfsOpenFlags::Reading as u32 != 0)
            && file.off != unsafe { (*lfs.cfg).block_size } {
        let off_val = file.pos;
        let oindex = unsafe { lfs_ctz_index(lfs, &off_val) };
        let mut noff = npos;
        let nindex = unsafe { lfs_ctz_index(lfs, &mut noff) };
        if oindex == nindex
                && noff >= file.cache.off
                && noff < file.cache.off + file.cache.size {
            file.pos = npos;
            file.off = noff;
            return npos as LfsSoff;
        }
    }

    // 事先写出所有内容，如果是只读模式则可能无操作
    let err = unsafe { lfs_file_flush(lfs, file) };
    if err != 0 {
        return err;
    }

    // 更新位置
    file.pos = npos;
    return npos as LfsSoff;
}

fn lfs_file_truncate_(lfs: &mut Lfs, file: &mut LfsFile, size: LfsOff) -> i32 {
    debug_assert!((file.flags & LfsOpenFlags::WrOnly as u32) == LfsOpenFlags::WrOnly as u32);

    if size > LFS_FILE_MAX {
        return LfsError::Inval as i32;
    }

    let pos = file.pos;
    let oldsize = lfs_file_size_(lfs, file);
    if size < oldsize {
        // revert to inline file?
        if size <= unsafe { (*lfs.cfg).inline_max } {
            // flush+seek to head
            let res = lfs_file_seek_(lfs, file, 0, LfsWhenceFlags::Set);
            if res < 0 {
                return res as i32;
            }

            // read our data into rcache temporarily
            lfs_cache_drop(lfs, &mut lfs.rcache);
            let res = lfs_file_flushedread(lfs, file, 
                lfs.rcache.buffer, size);
            if res < 0 {
                return res as i32;
            }

            file.ctz.head = LFS_BLOCK_INLINE;
            file.ctz.size = size;
            file.flags |= LfsOpenFlags::Dirty as u32 | LfsOpenFlags::Reading as u32 | LfsOpenFlags::Inline as u32;
            file.cache.block = file.ctz.head;
            file.cache.off = 0;
            file.cache.size = unsafe { (*lfs.cfg).cache_size };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    lfs.rcache.buffer, 
                    file.cache.buffer, 
                    size as usize
                );
            }
        } else {
            // need to flush since directly changing metadata
            let err = lfs_file_flush(lfs, file);
            if err != 0 {
                return err;
            }

            // lookup new head in ctz skip list
            let mut off: LfsOff = 0;
            let err = lfs_ctz_find(lfs, None, &mut file.cache,
                    file.ctz.head, file.ctz.size,
                    size-1, &mut file.block, &mut off);
            if err != 0 {
                return err;
            }

            // need to set pos/block/off consistently so seeking back to
            // the old position does not get confused
            file.pos = size;
            file.ctz.head = file.block;
            file.ctz.size = size;
            file.flags |= LfsOpenFlags::Dirty as u32 | LfsOpenFlags::Reading as u32;
        }
    } else if size > oldsize {
        // flush+seek if not already at end
        let res = lfs_file_seek_(lfs, file, 0, LfsWhenceFlags::End);
        if res < 0 {
            return res as i32;
        }

        // fill with zeros
        let zero: u8 = 0;
        while file.pos < size {
            let res = lfs_file_write_(lfs, file, &zero, 1);
            if res < 0 {
                return res as i32;
            }
        }
    }

    // restore pos
    let res = lfs_file_seek_(lfs, file, pos, LfsWhenceFlags::Set);
    if res < 0 {
        return res as i32;
    }

    0
}

/// 获取文件当前位置
///
/// # Arguments
///
/// * `lfs` - 文件系统指针
/// * `file` - 文件指针
///
/// # Returns
///
/// 当前文件位置
#[no_mangle]
pub unsafe fn lfs_file_tell_(lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    let _ = lfs; // 未使用的参数
    (*file).pos
}

/// 将文件指针重置到文件开头
///
/// 相当于调用 lfs_file_seek_(lfs, file, 0, LFS_SEEK_SET)
///
/// 返回:
///  0 成功
///  错误码 失败
fn lfs_file_rewind_(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let res = lfs_file_seek_(lfs, file, 0, LfsWhenceFlags::Set);
    if res < 0 {
        return res;
    }

    0
}

fn lfs_file_size_(lfs: &Lfs, file: &LfsFile) -> LfsSoff {
    // 使用lfs参数以避免未使用警告
    let _ = lfs;

    #[cfg(not(feature = "LFS_READONLY"))]
    {
        if (file.flags & LfsOpenFlags::Writing as u32) != 0 {
            return lfs_util::lfs_max(file.pos, file.ctz.size) as LfsSoff;
        }
    }

    file.ctz.size as LfsSoff
}

/// 获取路径指定文件的信息
/// 
/// # Arguments
/// * `lfs` - 文件系统实例
/// * `path` - 文件路径
/// * `info` - 信息存储结构
/// 
/// # Returns
/// * 成功返回0，失败返回错误码
fn lfs_stat_(lfs: *mut Lfs, path: *const u8, info: *mut LfsInfo) -> i32 {
    let mut cwd = LfsMdir::default();
    let tag = unsafe { lfs_dir_find(lfs, &mut cwd, &path, core::ptr::null_mut()) };
    if tag < 0 {
        return tag;
    }

    // 只允许目录末尾有斜杠
    unsafe {
        if !libc::strchr(path as *const i8, b'/' as i32).is_null() 
                && lfs_tag_type3(tag as LfsTag) != LfsType::Dir as u16 {
            return LfsError::NotDir as i32;
        }
    }

    unsafe { lfs_dir_getinfo(lfs, &cwd, lfs_tag_id(tag as LfsTag), info) }
}

fn lfs_remove_(lfs: &mut Lfs, path: &str) -> i32 {
    // deorphan if we haven't yet, needed at most once after poweron
    let err = lfs_fs_forceconsistency(lfs);
    if err != 0 {
        return err;
    }

    let mut cwd = LfsMdir::default();
    let tag = lfs_dir_find(lfs, &mut cwd, &path, None);
    if tag < 0 || lfs_tag_id(tag) == 0x3ff {
        return if tag < 0 { tag } else { LfsError::Inval as i32 };
    }

    let mut dir = LfsMlist {
        next: lfs.mlist,
        id: 0,
        type_: 0,
        m: LfsMdir::default(),
    };

    if lfs_tag_type3(tag) == LfsType::Dir as u16 {
        // must be empty before removal
        let mut pair = [0u32; 2];
        let res = lfs_dir_get(
            lfs,
            &cwd,
            LFS_MKTAG(0x700, 0x3ff, 0),
            LFS_MKTAG(LfsType::Struct as u16, lfs_tag_id(tag), 8),
            &mut pair,
        );
        if res < 0 {
            return res;
        }
        lfs_pair_fromle32(&mut pair);

        let err = lfs_dir_fetch(lfs, &mut dir.m, &pair);
        if err != 0 {
            return err;
        }

        if dir.m.count > 0 || dir.m.split {
            return LfsError::NotEmpty as i32;
        }

        // mark fs as orphaned
        let err = lfs_fs_preporphans(lfs, 1);
        if err != 0 {
            return err;
        }

        // I know it's crazy but yes, dir can be changed by our parent's
        // commit (if predecessor is child)
        dir.type_ = 0;
        dir.id = 0;
        lfs.mlist = &mut dir;
    }

    // delete the entry
    let err = lfs_dir_commit(
        lfs,
        &mut cwd,
        &[LfsMattr {
            tag: LFS_MKTAG(LfsType::Delete as u16, lfs_tag_id(tag), 0),
            buffer: core::ptr::null(),
        }],
    );
    if err != 0 {
        lfs.mlist = dir.next;
        return err;
    }

    lfs.mlist = dir.next;
    if lfs_tag_type3(tag) == LfsType::Dir as u16 {
        // fix orphan
        let err = lfs_fs_preporphans(lfs, -1);
        if err != 0 {
            return err;
        }

        let err = lfs_fs_pred(lfs, &dir.m.pair, &mut cwd);
        if err != 0 {
            return err;
        }

        let err = lfs_dir_drop(lfs, &mut cwd, &mut dir.m);
        if err != 0 {
            return err;
        }
    }

    0
}

fn lfs_rename_(lfs: &mut Lfs, oldpath: &str, newpath: &str) -> i32 {
    // deorphan if we haven't yet, needed at most once after poweron
    let err = lfs_fs_forceconsistency(lfs);
    if err != 0 {
        return err;
    }

    // find old entry
    let mut oldcwd = LfsMdir::default();
    let mut oldpath_ptr = oldpath;
    let oldtag = lfs_dir_find(lfs, &mut oldcwd, &mut oldpath_ptr, None);
    if oldtag < 0 || lfs_tag_id(oldtag) == 0x3ff {
        return if oldtag < 0 { oldtag } else { LfsError::Inval as i32 };
    }

    // find new entry
    let mut newcwd = LfsMdir::default();
    let mut newpath_ptr = newpath;
    let mut newid: u16 = 0;
    let prevtag = lfs_dir_find(lfs, &mut newcwd, &mut newpath_ptr, Some(&mut newid));
    if (prevtag < 0 || lfs_tag_id(prevtag) == 0x3ff) &&
        !(prevtag == LfsError::NoEnt as i32 && lfs_path_islast(newpath_ptr)) {
        return if prevtag < 0 { prevtag } else { LfsError::Inval as i32 };
    }

    // if we're in the same pair there's a few special cases...
    let samepair = lfs_pair_cmp(&oldcwd.pair, &newcwd.pair) == 0;
    let mut newoldid = lfs_tag_id(oldtag);

    let mut prevdir = LfsMlist {
        next: lfs.mlist,
        id: 0,
        type_: 0,
        m: LfsMdir::default(),
    };

    if prevtag == LfsError::NoEnt as i32 {
        // if we're a file, don't allow trailing slashes
        if lfs_path_isdir(newpath_ptr) &&
            lfs_tag_type3(oldtag) != LfsType::Dir as u16 {
            return LfsError::NotDir as i32;
        }

        // check that name fits
        let nlen = lfs_path_namelen(newpath_ptr);
        if nlen > lfs.name_max as usize {
            return LfsError::NameTooLong as i32;
        }

        // there is a small chance we are being renamed in the same
        // directory/ to an id less than our old id, the global update
        // to handle this is a bit messy
        if samepair && newid <= newoldid {
            newoldid += 1;
        }
    } else if lfs_tag_type3(prevtag) != lfs_tag_type3(oldtag) {
        return if lfs_tag_type3(prevtag) == LfsType::Dir as u16 {
            LfsError::IsDir as i32
        } else {
            LfsError::NotDir as i32
        };
    } else if samepair && newid == newoldid {
        // we're renaming to ourselves??
        return 0;
    } else if lfs_tag_type3(prevtag) == LfsType::Dir as u16 {
        // must be empty before removal
        let mut prevpair = [0u32; 2];
        let res = lfs_dir_get(
            lfs,
            &newcwd,
            lfs_mktag(0x700, 0x3ff, 0),
            lfs_mktag(LfsType::Struct as u16, newid, 8),
            &mut prevpair as *mut _ as *mut c_void
        );
        if res < 0 {
            return res;
        }
        lfs_pair_fromle32(&mut prevpair);

        // must be empty before removal
        let err = lfs_dir_fetch(lfs, &mut prevdir.m, &prevpair);
        if err != 0 {
            return err;
        }

        if prevdir.m.count > 0 || prevdir.m.split {
            return LfsError::NoTempty as i32;
        }

        // mark fs as orphaned
        let err = lfs_fs_preporphans(lfs, 1);
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
        lfs_fs_prepmove(lfs, newoldid, &oldcwd.pair);
    }

    // move over all attributes
    let attrs = [
        LfsMattr {
            tag: if prevtag != LfsError::NoEnt as i32 {
                lfs_mktag(LfsType::Delete as u16, newid, 0)
            } else {
                0
            },
            buffer: std::ptr::null(),
        },
        LfsMattr {
            tag: lfs_mktag(LfsType::Create as u16, newid, 0),
            buffer: std::ptr::null(),
        },
        LfsMattr {
            tag: lfs_mktag(lfs_tag_type3(oldtag), newid, lfs_path_namelen(newpath_ptr)),
            buffer: newpath_ptr as *const _ as *const c_void,
        },
        LfsMattr {
            tag: lfs_mktag(LfsType::FromMove as u16, newid, lfs_tag_id(oldtag)),
            buffer: &oldcwd as *const _ as *const c_void,
        },
        LfsMattr {
            tag: if samepair {
                lfs_mktag(LfsType::Delete as u16, newoldid, 0)
            } else {
                0
            },
            buffer: std::ptr::null(),
        },
    ];

    let err = lfs_dir_commit(lfs, &mut newcwd, &attrs, attrs.len() as i32);
    if err != 0 {
        lfs.mlist = prevdir.next;
        return err;
    }

    // let commit clean up after move (if we're different! otherwise move
    // logic already fixed it for us)
    if !samepair && lfs_gstate_hasmove(&lfs.gstate) {
        // prep gstate and delete move id
        lfs_fs_prepmove(lfs, 0x3ff, std::ptr::null());
        let attrs = [
            LfsMattr {
                tag: lfs_mktag(LfsType::Delete as u16, lfs_tag_id(oldtag), 0),
                buffer: std::ptr::null(),
            },
        ];
        
        let err = lfs_dir_commit(lfs, &mut oldcwd, &attrs, 1);
        if err != 0 {
            lfs.mlist = prevdir.next;
            return err;
        }
    }

    lfs.mlist = prevdir.next;
    if prevtag != LfsError::NoEnt as i32 && lfs_tag_type3(prevtag) == LfsType::Dir as u16 {
        // fix orphan
        let err = lfs_fs_preporphans(lfs, -1);
        if err != 0 {
            return err;
        }

        let err = lfs_fs_pred(lfs, &prevdir.m.pair, &mut newcwd);
        if err != 0 {
            return err;
        }

        let err = lfs_dir_drop(lfs, &mut newcwd, &mut prevdir.m);
        if err != 0 {
            return err;
        }
    }

    return 0;
}

/// 获取指定路径文件的属性
/// 
/// 参数:
/// - lfs: 文件系统实例
/// - path: 文件路径
/// - type_: 属性类型
/// - buffer: 存放属性数据的缓冲区
/// - size: 缓冲区大小
/// 
/// 返回:
/// - 成功时返回实际属性大小
/// - 失败时返回错误码
fn lfs_getattr_(lfs: &mut Lfs, path: &str, type_: u8, buffer: *mut c_void, size: LfsSize) -> LfsSsize {
    let mut cwd = LfsMdir::default();
    let tag = lfs_dir_find(lfs, &mut cwd, &path, None);
    if tag < 0 {
        return tag;
    }

    let mut id = lfs_tag_id(tag);
    if id == 0x3ff {
        // 根目录的特殊情况
        id = 0;
        let err = lfs_dir_fetch(lfs, &mut cwd, lfs.root);
        if err != 0 {
            return err;
        }
    }

    let tag = lfs_dir_get(
        lfs,
        &mut cwd,
        LFS_MKTAG(0x7ff, 0x3ff, 0),
        LFS_MKTAG(
            LfsType::UserAttr as u16 + type_ as u16,
            id,
            lfs_util::min(size, lfs.attr_max) as u16,
        ),
        buffer,
    );
    
    if tag < 0 {
        if tag == LfsError::NoEnt as i32 {
            return LfsError::NoAttr as i32;
        }

        return tag;
    }

    lfs_tag_size(tag)
}

/// Commits a user attribute to a file or directory
/// 
/// This function finds a file or directory by path and attaches a user attribute to it
fn lfs_commitattr(lfs: &mut Lfs, path: *const u8, 
                 type_: u8, buffer: *const c_void, size: LfsSize) -> LfsSsize {
    let mut cwd = LfsMdir::default();
    let tag = lfs_dir_find(lfs, &mut cwd, &path, None);
    if tag < 0 {
        return tag;
    }

    let mut id = lfs_tag_id(tag);
    if id == 0x3ff {
        // special case for root
        id = 0;
        let err = lfs_dir_fetch(lfs, &mut cwd, lfs.root);
        if err != 0 {
            return err;
        }
    }

    // LFS_MKATTRS macro equivalent in Rust:
    // Create a single attribute for the user attribute
    let attr = LfsMattr {
        tag: LFS_MKTAG(LfsType::UserAttr as u16 + type_ as u16, id, size),
        buffer: buffer,
    };
    
    // Commit the attribute to the directory
    lfs_dir_commit(lfs, &mut cwd, &attr, 1)
}

/// 设置文件属性
///
/// 根据指定的路径和类型设置属性。
/// 如果属性大小超过系统允许的最大值，返回错误。
fn lfs_setattr_(lfs: &mut Lfs, path: *const u8, type_: u8, buffer: *const c_void, size: LfsSize) -> i32 {
    if size > lfs.attr_max as u32 {
        return LfsError::NoSpc as i32;
    }

    lfs_commitattr(lfs, path, type_, buffer, size)
}

/// 移除文件属性
/// 
/// 从指定路径的文件中移除指定类型的属性
fn lfs_removeattr_(lfs: *mut Lfs, path: *const i8, type_: u8) -> i32 {
    unsafe {
        lfs_commitattr(lfs, path, type_, core::ptr::null(), 0x3ff)
    }
}

/// 初始化文件系统结构
///
/// 此函数为文件系统使用设置内存并根据配置验证参数
/// 但不应该在磁盘上实际修改任何东西
fn lfs_init(lfs: &mut Lfs, cfg: &LfsConfig) -> i32 {
    lfs.cfg = cfg;
    lfs.block_count = cfg.block_count;  // 可能为0
    let mut err = 0;

    #[cfg(feature = "LFS_MULTIVERSION")]
    {
        // 此驱动程序仅支持次版本 < 当前次版本
        debug_assert!(cfg.disk_version == 0 || (
                (0xffff & (cfg.disk_version >> 16)) == LFS_DISK_VERSION_MAJOR
                && (0xffff & (cfg.disk_version >> 0)) <= LFS_DISK_VERSION_MINOR));
    }

    // 检查bool是否是保真类型
    //
    // 注意这种失败最常见的原因是使用了C99之前的编译器，
    // littlefs当前不支持这种情况
    debug_assert!((0x80000000 as u32) != 0);

    // 检查所需的io函数是否已提供
    debug_assert!(lfs.cfg.read.is_some());
    #[cfg(not(feature = "LFS_READONLY"))]
    {
        debug_assert!(lfs.cfg.prog.is_some());
        debug_assert!(lfs.cfg.erase.is_some());
        debug_assert!(lfs.cfg.sync.is_some());
    }

    // 在执行任何算术逻辑之前，验证lfs-cfg大小是否正确初始化
    debug_assert!(lfs.cfg.read_size != 0);
    debug_assert!(lfs.cfg.prog_size != 0);
    debug_assert!(lfs.cfg.cache_size != 0);

    // 检查块大小是缓存大小的倍数，缓存大小是prog和read大小的倍数
    debug_assert!(lfs.cfg.cache_size % lfs.cfg.read_size == 0);
    debug_assert!(lfs.cfg.cache_size % lfs.cfg.prog_size == 0);
    debug_assert!(lfs.cfg.block_size % lfs.cfg.cache_size == 0);

    // 检查块大小是否足够大以容纳所有ctz指针
    debug_assert!(lfs.cfg.block_size >= 128);
    // 这是所有ctz指针的精确计算，如果此失败而上面的简单断言未失败，则数学计算必须有问题
    debug_assert!(4 * lfs_util::lfs_npw2(0xffffffff / (lfs.cfg.block_size - 2 * 4))
            <= lfs.cfg.block_size);

    // block_cycles = 0 不再支持
    //
    // block_cycles是littlefs在擦除周期之前将元数据日志逐出作为磨损均衡的一部分的次数。
    // 建议值在100-1000范围内，或设置block_cycles为-1以禁用块级磨损均衡。
    debug_assert!(lfs.cfg.block_cycles != 0);

    // 检查compact_thresh是否有意义
    //
    // 元数据不能被压缩到低于block_size/2，并且元数据不能超过block_size
    debug_assert!(lfs.cfg.compact_thresh == 0
            || lfs.cfg.compact_thresh >= lfs.cfg.block_size / 2);
    debug_assert!(lfs.cfg.compact_thresh == (!0 as LfsSize)
            || lfs.cfg.compact_thresh <= lfs.cfg.block_size);

    // 检查metadata_max是read_size和prog_size的倍数，
    // 且是block_size的因子
    debug_assert!(lfs.cfg.metadata_max == 0
            || lfs.cfg.metadata_max % lfs.cfg.read_size == 0);
    debug_assert!(lfs.cfg.metadata_max == 0
            || lfs.cfg.metadata_max % lfs.cfg.prog_size == 0);
    debug_assert!(lfs.cfg.metadata_max == 0
            || lfs.cfg.block_size % lfs.cfg.metadata_max == 0);

    // 设置读缓存
    if !lfs.cfg.read_buffer.is_null() {
        lfs.rcache.buffer = lfs.cfg.read_buffer as *mut u8;
    } else {
        lfs.rcache.buffer = lfs_util::lfs_malloc(lfs.cfg.cache_size) as *mut u8;
        if lfs.rcache.buffer.is_null() {
            err = LfsError::NoMem as i32;
            goto!(cleanup);
        }
    }

    // 设置程序缓存
    if !lfs.cfg.prog_buffer.is_null() {
        lfs.pcache.buffer = lfs.cfg.prog_buffer as *mut u8;
    } else {
        lfs.pcache.buffer = lfs_util::lfs_malloc(lfs.cfg.cache_size) as *mut u8;
        if lfs.pcache.buffer.is_null() {
            err = LfsError::NoMem as i32;
            goto!(cleanup);
        }
    }

    // 清零以避免信息泄漏
    lfs_cache_zero(lfs, &mut lfs.rcache);
    lfs_cache_zero(lfs, &mut lfs.pcache);

    // 设置前瞻缓冲区，注意挂载在我们建立一个合理的伪随机种子后完成初始化
    debug_assert!(lfs.cfg.lookahead_size > 0);
    if !lfs.cfg.lookahead_buffer.is_null() {
        lfs.lookahead.buffer = lfs.cfg.lookahead_buffer as *mut u8;
    } else {
        lfs.lookahead.buffer = lfs_util::lfs_malloc(lfs.cfg.lookahead_size) as *mut u8;
        if lfs.lookahead.buffer.is_null() {
            err = LfsError::NoMem as i32;
            goto!(cleanup);
        }
    }

    // 检查大小限制是否合理
    debug_assert!(lfs.cfg.name_max <= LFS_NAME_MAX);
    lfs.name_max = lfs.cfg.name_max;
    if lfs.name_max == 0 {
        lfs.name_max = LFS_NAME_MAX as LfsSize;
    }

    debug_assert!(lfs.cfg.file_max <= LFS_FILE_MAX);
    lfs.file_max = lfs.cfg.file_max;
    if lfs.file_max == 0 {
        lfs.file_max = LFS_FILE_MAX;
    }

    debug_assert!(lfs.cfg.attr_max <= LFS_ATTR_MAX as LfsSize);
    lfs.attr_max = lfs.cfg.attr_max;
    if lfs.attr_max == 0 {
        lfs.attr_max = LFS_ATTR_MAX as LfsSize;
    }

    debug_assert!(lfs.cfg.metadata_max <= lfs.cfg.block_size);

    debug_assert!(lfs.cfg.inline_max == (!0 as LfsSize)
            || lfs.cfg.inline_max <= lfs.cfg.cache_size);
    debug_assert!(lfs.cfg.inline_max == (!0 as LfsSize)
            || lfs.cfg.inline_max <= lfs.attr_max);
    debug_assert!(lfs.cfg.inline_max == (!0 as LfsSize)
            || lfs.cfg.inline_max <= ((if lfs.cfg.metadata_max != 0 {
                lfs.cfg.metadata_max
            } else {
                lfs.cfg.block_size
            }) / 8));
    lfs.inline_max = lfs.cfg.inline_max;
    if lfs.inline_max == (!0 as LfsSize) {
        lfs.inline_max = 0;
    } else if lfs.inline_max == 0 {
        lfs.inline_max = lfs_util::lfs_min(
                lfs.cfg.cache_size,
                lfs_util::lfs_min(
                    lfs.attr_max,
                    ((if lfs.cfg.metadata_max != 0 {
                        lfs.cfg.metadata_max
                    } else {
                        lfs.cfg.block_size
                    }) / 8)));
    }

    // 设置默认状态
    lfs.root[0] = LFS_BLOCK_NULL;
    lfs.root[1] = LFS_BLOCK_NULL;
    lfs.mlist = core::ptr::null_mut();
    lfs.seed = 0;
    lfs.gdisk = LfsGstate { tag: 0, pair: [0, 0] };
    lfs.gstate = LfsGstate { tag: 0, pair: [0, 0] };
    lfs.gdelta = LfsGstate { tag: 0, pair: [0, 0] };
    #[cfg(feature = "LFS_MIGRATE")]
    {
        lfs.lfs1 = core::ptr::null_mut();
    }

    return 0;

    // 标签：cleanup
    lfs_deinit(lfs);
    return err;
}

/// 释放LittleFS文件系统结构体中分配的内存
pub(crate) fn lfs_deinit(lfs: &mut Lfs) -> i32 {
    // 释放已分配的内存
    unsafe {
        // 如果没有提供读缓冲区，则释放内部分配的缓冲区
        if (*lfs.cfg).read_buffer.is_null() {
            lfs_util::lfs_free(lfs.rcache.buffer as *mut c_void);
        }

        // 如果没有提供写入缓冲区，则释放内部分配的缓冲区
        if (*lfs.cfg).prog_buffer.is_null() {
            lfs_util::lfs_free(lfs.pcache.buffer as *mut c_void);
        }

        // 如果没有提供前瞻缓冲区，则释放内部分配的缓冲区
        if (*lfs.cfg).lookahead_buffer.is_null() {
            lfs_util::lfs_free(lfs.lookahead.buffer as *mut c_void);
        }
    }

    0
}

/// 格式化文件系统
pub fn lfs_format_(lfs: *mut Lfs, cfg: *const LfsConfig) -> i32 {
    let mut err = 0;
    {
        err = unsafe { lfs_init(lfs, cfg) };
        if err != 0 {
            return err;
        }

        unsafe {
            assert!((*cfg).block_count != 0);

            // 创建空闲前瞻
            core::ptr::write_bytes((*lfs).lookahead.buffer, 0, (*(*lfs).cfg).lookahead_size as usize);
            (*lfs).lookahead.start = 0;
            (*lfs).lookahead.size = core::cmp::min(
                8 * (*(*lfs).cfg).lookahead_size,
                (*lfs).block_count
            );
            (*lfs).lookahead.next = 0;
            lfs_alloc_ckpoint(lfs);

            // 创建根目录
            let mut root = core::mem::MaybeUninit::<LfsMdir>::uninit().assume_init();
            err = lfs_dir_alloc(lfs, &mut root);
            if err != 0 {
                goto!(cleanup);
            }

            // 写入一个超级块
            let mut superblock = LfsSuperblock {
                version: lfs_fs_disk_version(lfs),
                block_size: (*(*lfs).cfg).block_size,
                block_count: (*lfs).block_count,
                name_max: (*lfs).name_max,
                file_max: (*lfs).file_max,
                attr_max: (*lfs).attr_max,
            };

            lfs_superblock_tole32(&mut superblock);
            
            // 使用LFS_MKATTRS宏构造属性
            let attrs = [
                LfsMattr {
                    tag: LFS_MKTAG(LfsType::Create as u16, 0, 0),
                    buffer: core::ptr::null(),
                },
                LfsMattr {
                    tag: LFS_MKTAG(LfsType::SuperBlock as u16, 0, 8),
                    buffer: "littlefs".as_ptr() as *const c_void,
                },
                LfsMattr {
                    tag: LFS_MKTAG(LfsType::InlineStruct as u16, 0, core::mem::size_of::<LfsSuperblock>() as u32),
                    buffer: &superblock as *const _ as *const c_void,
                },
            ];
            
            err = lfs_dir_commit(lfs, &mut root, attrs.as_ptr(), attrs.len() as i32);
            if err != 0 {
                goto!(cleanup);
            }

            // 强制压缩以防止意外挂载可能存在于磁盘上的任何旧版本littlefs
            root.erased = false;
            err = lfs_dir_commit(lfs, &mut root, core::ptr::null(), 0);
            if err != 0 {
                goto!(cleanup);
            }

            // 完整性检查，确保fetch功能正常
            err = lfs_dir_fetch(lfs, &mut root, &[0, 1] as *const LfsBlock);
            if err != 0 {
                goto!(cleanup);
            }
        }
    }

    // 标签：cleanup
    unsafe { lfs_deinit(lfs) };
    err
}

/// 使用Brent算法检测目录循环
fn lfs_tortoise_detectcycles(
    dir: &LfsMdir, 
    tortoise: &mut LfsTortoise,
) -> i32 {
    // 使用Brent算法检测循环
    if lfs_util::lfs_pair_issync(&dir.tail, &tortoise.pair) {
        lfs_util::LFS_WARN!("Cycle detected in tail list");
        return LfsError::Corrupt as i32;
    }
    
    if tortoise.i == tortoise.period {
        tortoise.pair[0] = dir.tail[0];
        tortoise.pair[1] = dir.tail[1];
        tortoise.i = 0;
        tortoise.period *= 2;
    }
    
    tortoise.i += 1;

    LfsError::Ok as i32
}

fn lfs_mount_(lfs: &mut Lfs, cfg: &LfsConfig) -> i32 {
    let err = lfs_init(lfs, cfg);
    if err != 0 {
        return err;
    }

    // scan directory blocks for superblock and any global updates
    let mut dir = LfsMdir {
        tail: [0, 1],
        pair: [0, 0],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
    };
    let mut tortoise = LfsTortoise {
        pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
        i: 1,
        period: 1,
    };

    while !lfs_pair_isnull(dir.tail) {
        let cycle_err = lfs_tortoise_detectcycles(&mut dir, &mut tortoise);
        if cycle_err < 0 {
            goto_cleanup!(err = cycle_err);
        }

        // fetch next block in tail list
        let tag = lfs_dir_fetchmatch(
            lfs,
            &mut dir,
            dir.tail,
            LFS_MKTAG(0x7ff, 0x3ff, 0),
            LFS_MKTAG(LfsType::SuperBlock as u16, 0, 8),
            core::ptr::null(),
            Some(lfs_dir_find_match),
            &LfsDirFindMatch {
                lfs,
                name: b"littlefs".as_ptr() as *const c_void,
                size: 8,
            },
        );
        if tag < 0 {
            goto_cleanup!(err = tag);
        }

        // has superblock?
        if tag != 0 && !lfs_tag_isdelete(tag as u32) {
            // update root
            lfs.root[0] = dir.pair[0];
            lfs.root[1] = dir.pair[1];

            // grab superblock
            let mut superblock = LfsSuperblock {
                version: 0,
                block_size: 0,
                block_count: 0,
                name_max: 0,
                file_max: 0,
                attr_max: 0,
            };
            let tag = lfs_dir_get(
                lfs,
                &mut dir,
                LFS_MKTAG(0x7ff, 0x3ff, 0),
                LFS_MKTAG(LfsType::InlineStruct as u16, 0, core::mem::size_of::<LfsSuperblock>() as u32),
                &mut superblock as *mut _ as *mut c_void,
            );
            if tag < 0 {
                goto_cleanup!(err = tag);
            }
            lfs_superblock_fromle32(&mut superblock);

            // check version
            let major_version = 0xffff & (superblock.version >> 16);
            let minor_version = 0xffff & (superblock.version >> 0);
            if major_version != lfs_fs_disk_version_major(lfs)
                || minor_version > lfs_fs_disk_version_minor(lfs)
            {
                LFS_ERROR!(
                    "Invalid version v{}.{} != v{}.{}",
                    major_version,
                    minor_version,
                    lfs_fs_disk_version_major(lfs),
                    lfs_fs_disk_version_minor(lfs)
                );
                goto_cleanup!(err = LfsError::Inval as i32);
            }

            // found older minor version? set an in-device only bit in the
            // gstate so we know we need to rewrite the superblock before
            // the first write
            let mut needssuperblock = false;
            if minor_version < lfs_fs_disk_version_minor(lfs) {
                LFS_DEBUG!(
                    "Found older minor version v{}.{} < v{}.{}",
                    major_version,
                    minor_version,
                    lfs_fs_disk_version_major(lfs),
                    lfs_fs_disk_version_minor(lfs)
                );
                needssuperblock = true;
            }
            // note this bit is reserved on disk, so fetching more gstate
            // will not interfere here
            lfs_fs_prepsuperblock(lfs, needssuperblock);

            // check superblock configuration
            if superblock.name_max != 0 {
                if superblock.name_max > lfs.name_max {
                    LFS_ERROR!(
                        "Unsupported name_max ({} > {})",
                        superblock.name_max,
                        lfs.name_max
                    );
                    goto_cleanup!(err = LfsError::Inval as i32);
                }

                lfs.name_max = superblock.name_max;
            }

            if superblock.file_max != 0 {
                if superblock.file_max > lfs.file_max {
                    LFS_ERROR!(
                        "Unsupported file_max ({} > {})",
                        superblock.file_max,
                        lfs.file_max
                    );
                    goto_cleanup!(err = LfsError::Inval as i32);
                }

                lfs.file_max = superblock.file_max;
            }

            if superblock.attr_max != 0 {
                if superblock.attr_max > lfs.attr_max {
                    LFS_ERROR!(
                        "Unsupported attr_max ({} > {})",
                        superblock.attr_max,
                        lfs.attr_max
                    );
                    goto_cleanup!(err = LfsError::Inval as i32);
                }

                lfs.attr_max = superblock.attr_max;

                // we also need to update inline_max in case attr_max changed
                lfs.inline_max = lfs_min(lfs.inline_max, lfs.attr_max);
            }

            // this is where we get the block_count from disk if block_count=0
            if unsafe { (*lfs.cfg).block_count != 0 }
                && superblock.block_count != unsafe { (*lfs.cfg).block_count }
            {
                LFS_ERROR!(
                    "Invalid block count ({} != {})",
                    superblock.block_count,
                    unsafe { (*lfs.cfg).block_count }
                );
                goto_cleanup!(err = LfsError::Inval as i32);
            }

            lfs.block_count = superblock.block_count;

            if superblock.block_size != unsafe { (*lfs.cfg).block_size } {
                LFS_ERROR!(
                    "Invalid block size ({} != {})",
                    superblock.block_size,
                    unsafe { (*lfs.cfg).block_size }
                );
                goto_cleanup!(err = LfsError::Inval as i32);
            }
        }

        // has gstate?
        let gstate_err = lfs_dir_getgstate(lfs, &mut dir, &mut lfs.gstate);
        if gstate_err != 0 {
            goto_cleanup!(err = gstate_err);
        }
    }

    // update littlefs with gstate
    if !lfs_gstate_iszero(&lfs.gstate) {
        LFS_DEBUG!(
            "Found pending gstate 0x{:08x}{:08x}{:08x}",
            lfs.gstate.tag,
            lfs.gstate.pair[0],
            lfs.gstate.pair[1]
        );
    }
    lfs.gstate.tag += (!lfs_tag_isvalid(lfs.gstate.tag)) as u32;
    lfs.gdisk = lfs.gstate;

    // setup free lookahead, to distribute allocations uniformly across
    // boots, we start the allocator at a random location
    lfs.lookahead.start = lfs.seed % lfs.block_count;
    lfs_alloc_drop(lfs);

    return 0;

    // 处理清理部分
    'cleanup: {
        lfs_unmount_(lfs);
        return err;
    }
}

/// 卸载文件系统，内部实现
fn lfs_unmount_(lfs: *mut Lfs) -> i32 {
    lfs_deinit(lfs)
}

fn lfs_fs_stat_(lfs: &mut Lfs, fsinfo: &mut LfsFsinfo) -> LfsSoff {
    // if the superblock is up-to-date, we must be on the most recent
    // minor version of littlefs
    if !lfs_gstate_needssuperblock(&lfs.gstate) {
        fsinfo.disk_version = lfs_fs_disk_version(lfs);
    // otherwise we need to read the minor version on disk
    } else {
        // fetch the superblock
        let mut dir = LfsMdir {
            pair: [0, 0],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0, 0],
        };
        
        let err = lfs_dir_fetch(lfs, &mut dir, lfs.root);
        if err < 0 {
            return err;
        }

        let mut superblock = LfsSuperblock {
            version: 0,
            block_size: 0,
            block_count: 0,
            name_max: 0,
            file_max: 0,
            attr_max: 0,
        };
        
        let tag = lfs_dir_get(
            lfs, 
            &dir, 
            LFS_MKTAG(0x7ff, 0x3ff, 0),
            LFS_MKTAG(LfsType::InlineStruct as u16, 0, core::mem::size_of::<LfsSuperblock>() as u16),
            &mut superblock as *mut _ as *mut c_void
        );
        
        if tag < 0 {
            return tag;
        }
        
        lfs_superblock_fromle32(&mut superblock);

        // read the on-disk version
        fsinfo.disk_version = superblock.version;
    }

    // filesystem geometry
    fsinfo.block_size = unsafe { (*lfs.cfg).block_size };
    fsinfo.block_count = lfs.block_count;

    // other on-disk configuration, we cache all of these for internal use
    fsinfo.name_max = lfs.name_max;
    fsinfo.file_max = lfs.file_max;
    fsinfo.attr_max = lfs.attr_max;

    0
}

/// 遍历文件系统中的所有块
///
/// 此函数将递归遍历文件系统并对每个块调用回调函数
/// 如果includeorphans为true，则还会包括孤立块
pub fn lfs_fs_traverse_(
    lfs: *mut Lfs,
    cb: Option<unsafe extern "C" fn(data: *mut c_void, block: LfsBlock) -> i32>,
    data: *mut c_void,
    includeorphans: bool,
) -> i32 {
    // 迭代元数据对
    let mut dir = LfsMdir {
        pair: [0, 0],
        rev: 0,
        off: 0,
        etag: 0,
        count: 0,
        erased: false,
        split: false,
        tail: [0, 1],
    };

    #[cfg(feature = "LFS_MIGRATE")]
    {
        // 在迁移期间也考虑v1块
        unsafe {
            if !(*lfs).lfs1.is_null() {
                let err = lfs1_traverse(lfs, cb, data);
                if err != 0 {
                    return err;
                }

                dir.tail[0] = (*lfs).root[0];
                dir.tail[1] = (*lfs).root[1];
            }
        }
    }

    let mut tortoise = LfsTortoise {
        pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
        i: 1,
        period: 1,
    };
    
    let mut err = LfsError::Ok as i32;
    
    unsafe {
        while !lfs_pair_isnull(dir.tail) {
            err = lfs_tortoise_detectcycles(&mut dir, &mut tortoise);
            if err < 0 {
                return LfsError::Corrupt as i32;
            }

            for i in 0..2 {
                if let Some(callback) = cb {
                    err = callback(data, dir.tail[i]);
                    if err != 0 {
                        return err;
                    }
                }
            }

            // 迭代目录中的ID
            err = lfs_dir_fetch(lfs, &mut dir, dir.tail);
            if err != 0 {
                return err;
            }

            for id in 0..dir.count {
                let mut ctz = LfsCtz { head: 0, size: 0 };
                let tag = lfs_dir_get(
                    lfs,
                    &mut dir,
                    lfs_util::LFS_MKTAG(0x700, 0x3ff, 0),
                    lfs_util::LFS_MKTAG(LfsType::Struct as u16, id, core::mem::size_of::<LfsCtz>() as u16),
                    &mut ctz as *mut _ as *mut c_void,
                );
                
                if tag < 0 {
                    if tag == LfsError::NoEnt as i32 {
                        continue;
                    }
                    return tag;
                }
                
                lfs_ctz_fromle32(&mut ctz);

                if lfs_util::lfs_tag_type3(tag as u32) == LfsType::CtzStruct as u16 {
                    err = lfs_ctz_traverse(
                        lfs,
                        core::ptr::null_mut(),
                        &mut (*lfs).rcache,
                        ctz.head,
                        ctz.size,
                        cb,
                        data,
                    );
                    if err != 0 {
                        return err;
                    }
                } else if includeorphans && 
                          lfs_util::lfs_tag_type3(tag as u32) == LfsType::DirStruct as u16 {
                    for i in 0..2 {
                        if let Some(callback) = cb {
                            let block_ptr = &ctz.head as *const LfsBlock;
                            err = callback(data, *block_ptr.add(i));
                            if err != 0 {
                                return err;
                            }
                        }
                    }
                }
            }
        }

        #[cfg(not(feature = "LFS_READONLY"))]
        {
            // 迭代所有打开的文件
            let mut f = (*lfs).mlist as *mut LfsFile;
            while !f.is_null() {
                if (*f).type_ != LfsType::Reg as u8 {
                    f = (*f).next as *mut LfsFile;
                    continue;
                }

                if ((*f).flags & LfsOpenFlags::Dirty as u32 != 0) && 
                   ((*f).flags & LfsOpenFlags::Inline as u32 == 0) {
                    err = lfs_ctz_traverse(
                        lfs,
                        &mut (*f).cache,
                        &mut (*lfs).rcache,
                        (*f).ctz.head,
                        (*f).ctz.size,
                        cb,
                        data,
                    );
                    if err != 0 {
                        return err;
                    }
                }

                if ((*f).flags & LfsOpenFlags::Writing as u32 != 0) && 
                   ((*f).flags & LfsOpenFlags::Inline as u32 == 0) {
                    err = lfs_ctz_traverse(
                        lfs,
                        &mut (*f).cache,
                        &mut (*lfs).rcache,
                        (*f).block,
                        (*f).pos,
                        cb,
                        data,
                    );
                    if err != 0 {
                        return err;
                    }
                }

                f = (*f).next as *mut LfsFile;
            }
        }
    }

    0
}

/// 查找给定元数据目录对的父目录
///
/// 针对给定的元数据目录对，搜索文件系统以查找其父目录。
/// 使用龟兔算法检测目录链中的循环
///
/// @param lfs    文件系统
/// @param pair   要查找父目录的元数据目录对
/// @param pdir   用于存储找到的父目录的引用
/// @return       成功时返回0，错误时返回负值
fn lfs_fs_pred(lfs: &mut Lfs, pair: &[LfsBlock; 2], pdir: &mut LfsMdir) -> i32 {
    // 初始化循环检测
    pdir.tail[0] = 0;
    pdir.tail[1] = 1;
    let mut tortoise = LfsTortoise {
        pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
        i: 1,
        period: 1,
    };
    
    let mut err = LfsError::Ok as i32;
    
    // 遍历所有目录项
    while !lfs_pair_isnull(&pdir.tail) {
        // 检测循环
        err = lfs_tortoise_detectcycles(pdir, &mut tortoise);
        if err < 0 {
            return LfsError::Corrupt as i32;
        }

        // 检查是否找到目标对
        if lfs_pair_cmp(&pdir.tail, pair) == 0 {
            return 0;
        }

        // 获取下一个目录
        err = lfs_dir_fetch(lfs, pdir, &pdir.tail);
        if err != 0 {
            return err;
        }
    }

    // 未找到目标对
    return LfsError::NoEnt as i32;
}

fn lfs_fs_parent_match(data: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32 {
    let find = unsafe { &mut *(data as *mut LfsFsParentMatch) };
    let lfs = unsafe { &mut *find.lfs };
    let disk = unsafe { &*(buffer as *const LfsDiskoff) };
    
    // 声明child数组并读取
    let mut child = [0u32; 2];
    let err = unsafe {
        lfs_bd_read(
            lfs,
            &mut lfs.pcache,
            &mut lfs.rcache,
            (*lfs.cfg).block_size,
            disk.block,
            disk.off,
            child.as_mut_ptr() as *mut c_void,
            std::mem::size_of::<[LfsBlock; 2]>() as LfsSize
        )
    };
    
    if err != 0 {
        return err;
    }
    
    // 转换小端序为本地序
    lfs_pair_fromle32(&mut child);
    
    // 比较配对块，如果匹配则返回LFS_CMP_EQ，否则返回LFS_CMP_LT
    if lfs_pair_cmp(&child, &find.pair) == 0 {
        LfsCmp::Eq as i32
    } else {
        LfsCmp::Lt as i32
    }
}

/// 在文件系统中查找给定元数据目录对的父目录
static fn lfs_fs_parent(lfs: *mut Lfs, pair: &[LfsBlock; 2], parent: *mut LfsMdir) -> LfsStag {
    // 使用fetchmatch和回调查找对
    unsafe {
        (*parent).tail[0] = 0;
        (*parent).tail[1] = 1;
        let mut tortoise = LfsTortoise {
            pair: [LFS_BLOCK_NULL, LFS_BLOCK_NULL],
            i: 1,
            period: 1,
        };
        
        let mut err = LfsError::Ok as i32;
        while !lfs_pair_isnull(&(*parent).tail) {
            err = lfs_tortoise_detectcycles(parent, &mut tortoise);
            if err < 0 {
                return err;
            }

            let match_data = LfsFsParentMatch {
                lfs,
                pair: [pair[0], pair[1]],
            };
            
            let tag = lfs_dir_fetchmatch(
                lfs, 
                parent, 
                &(*parent).tail,
                lfs_tag_makemask(0x7ff, 0, 0x3ff),
                lfs_tag_make(LfsType::DirStruct as u16, 0, 8),
                core::ptr::null(),
                Some(lfs_fs_parent_match),
                &match_data as *const _ as *mut c_void
            );
            
            if tag != 0 && tag != LfsError::NoEnt as i32 {
                return tag;
            }
        }

        return LfsError::NoEnt as i32;
    }
}

/// 准备超级块的设置
/// 
/// 设置全局状态标签中的需要超级块标志位
fn lfs_fs_prepsuperblock(lfs: &mut Lfs, needssuperblock: bool) {
    lfs.gstate.tag = (lfs.gstate.tag & !LFS_MKTAG(0, 0, 0x200))
        | ((needssuperblock as u32) << 9);
}

/// 预处理孤立节点
/// 
/// 调整文件系统的全局状态标签，以反映孤立节点的存在
pub fn lfs_fs_preporphans(lfs: &mut Lfs, orphans: i8) -> i32 {
    debug_assert!(lfs_tag_size(lfs.gstate.tag) > 0x000 || orphans >= 0);
    debug_assert!(lfs_tag_size(lfs.gstate.tag) < 0x1ff || orphans <= 0);
    
    lfs.gstate.tag += orphans as u32;
    lfs.gstate.tag = (lfs.gstate.tag & !LFS_MKTAG(0x800, 0, 0)) |
        ((lfs_gstate_hasorphans(&lfs.gstate) as u32) << 31);

    0
}

/// 准备文件系统中文件的移动操作
/// 
/// 更新全局状态(gstate)的标记和对信息，为文件移动做准备
fn lfs_fs_prepmove(lfs: &mut Lfs, id: u16, pair: &[LfsBlock; 2]) {
    lfs.gstate.tag = (lfs.gstate.tag & !LFS_MKTAG(0x7ff, 0x3ff, 0)) |
        ((id != 0x3ff) as u32 * LFS_MKTAG(LfsType::Delete as u16, id, 0));
    lfs.gstate.pair[0] = if id != 0x3ff { pair[0] } else { 0 };
    lfs.gstate.pair[1] = if id != 0x3ff { pair[1] } else { 0 };
}

/// 重写超级块，如果不需要则跳过
fn lfs_fs_desuperblock(lfs: &mut Lfs) -> i32 {
    if !lfs_gstate_needssuperblock(&lfs.gstate) {
        return 0;
    }

    LFS_DEBUG!("Rewriting superblock {{0x{:x}, 0x{:x}}}",
            lfs.root[0],
            lfs.root[1]);

    let mut root = LfsMdir::default();
    let err = lfs_dir_fetch(lfs, &mut root, lfs.root);
    if err != 0 {
        return err;
    }

    // 写入新的超级块
    let mut superblock = LfsSuperblock {
        version: lfs_fs_disk_version(lfs),
        block_size: unsafe { (*lfs.cfg).block_size },
        block_count: lfs.block_count,
        name_max: lfs.name_max,
        file_max: lfs.file_max,
        attr_max: lfs.attr_max,
    };

    lfs_superblock_tole32(&mut superblock);
    let err = lfs_dir_commit(lfs, &mut root, &[
        LfsMattr {
            tag: LFS_MKTAG!(LfsType::InlineStruct, 0, core::mem::size_of::<LfsSuperblock>() as u32),
            buffer: &superblock as *const _ as *const c_void,
        }
    ]);
    if err != 0 {
        return err;
    }

    lfs_fs_prepsuperblock(lfs, false);
    0
}

/// 处理恢复移动操作
///
/// 如果存在一个移动操作被中断，则修复它
fn lfs_fs_demove(lfs: &mut Lfs) -> LfsSoff {
    // 如果没有需要处理的移动，直接返回
    if !lfs_gstate_hasmove(&lfs.gdisk) {
        return 0;
    }

    // 修复错误的移动
    lfs_debug!(
        "Fixing move {0x%x, 0x%x} 0x%x",
        lfs.gdisk.pair[0],
        lfs.gdisk.pair[1],
        lfs_tag_id(lfs.gdisk.tag)
    );

    // 此时不支持其他gstate，如果找到了其他内容，那么很可能是gstate计算出错
    lfs_assert!(lfs_tag_type3(lfs.gdisk.tag) == LfsType::Delete as u32);

    // 获取并删除被移动的条目
    let mut movedir = LfsMdir::default();
    let err = lfs_dir_fetch(lfs, &mut movedir, lfs.gdisk.pair);
    if err < 0 {
        return err;
    }

    // 准备gstate并删除move id
    let moveid = lfs_tag_id(lfs.gdisk.tag);
    lfs_fs_prepmove(lfs, 0x3ff, core::ptr::null());
    
    // 创建一个属性删除标记并提交到目录
    let attrs = [LfsMattr {
        tag: LFS_MKTAG!(LfsType::Delete, moveid, 0),
        buffer: core::ptr::null(),
    }];
    
    let err = lfs_dir_commit(lfs, &mut movedir, &attrs);
    if err < 0 {
        return err;
    }

    0
}

fn lfs_fs_deorphan(lfs: &mut Lfs, powerloss: bool) -> i32 {
    if !lfs_gstate_hasorphans(&lfs.gstate) {
        return 0;
    }

    // Check for orphans in two separate passes:
    // - 1 for half-orphans (relocations)
    // - 2 for full-orphans (removes/renames)
    //
    // Two separate passes are needed as half-orphans can contain outdated
    // references to full-orphans, effectively hiding them from the deorphan
    // search.
    //
    let mut pass = 0;
    while pass < 2 {
        // Fix any orphans
        let mut pdir = LfsMdir {
            split: true,
            tail: [0, 1],
            pair: [0, 0],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
        };
        let mut dir: LfsMdir;
        let mut moreorphans = false;

        // iterate over all directory directory entries
        while !lfs_pair_isnull(pdir.tail) {
            let err = lfs_dir_fetch(lfs, &mut dir, pdir.tail);
            if err != 0 {
                return err;
            }

            // check head blocks for orphans
            if !pdir.split {
                // check if we have a parent
                let mut parent = LfsMdir::default();
                let tag = lfs_fs_parent(lfs, pdir.tail, &mut parent);
                if tag < 0 && tag != LfsError::NoEnt as i32 {
                    return tag;
                }

                if pass == 0 && tag != LfsError::NoEnt as i32 {
                    let mut pair = [0, 0];
                    let state = lfs_dir_get(
                        lfs, 
                        &parent,
                        LFS_MKTAG(0x7ff, 0x3ff, 0),
                        tag, 
                        &mut pair
                    );
                    if state < 0 {
                        return state;
                    }
                    lfs_pair_fromle32(&mut pair);

                    if !lfs_pair_issync(&pair, &pdir.tail) {
                        // we have desynced
                        LFS_DEBUG!(
                            "Fixing half-orphan {0:#x}, {0:#x} -> {0:#x}, {0:#x}",
                            pdir.tail[0], pdir.tail[1], pair[0], pair[1]
                        );

                        // fix pending move in this pair? this looks like an
                        // optimization but is in fact _required_ since
                        // relocating may outdate the move.
                        let mut moveid = 0x3ff;
                        if lfs_gstate_hasmovehere(&lfs.gstate, &pdir.pair) {
                            moveid = lfs_tag_id(lfs.gstate.tag);
                            LFS_DEBUG!(
                                "Fixing move while fixing orphans {0:#x}, {0:#x} {0:#x}\n",
                                pdir.pair[0], pdir.pair[1], moveid
                            );
                            lfs_fs_prepmove(lfs, 0x3ff, std::ptr::null_mut());
                        }

                        lfs_pair_tole32(&mut pair);
                        let attrs = [
                            LfsMattr {
                                tag: if moveid != 0x3ff {
                                    LFS_MKTAG(LfsType::Delete as u16, moveid, 0)
                                } else {
                                    0
                                },
                                buffer: std::ptr::null(),
                            },
                            LfsMattr {
                                tag: LFS_MKTAG(LfsType::SoftTail as u16, 0x3ff, 8),
                                buffer: pair.as_ptr() as *const c_void,
                            },
                        ];
                        
                        let state = lfs_dir_orphaningcommit(lfs, &mut pdir, &attrs);
                        lfs_pair_fromle32(&mut pair);
                        if state < 0 {
                            return state;
                        }

                        // did our commit create more orphans?
                        if state == LfsOk::Orphaned as i32 {
                            moreorphans = true;
                        }

                        // refetch tail
                        continue;
                    }
                }

                // note we only check for full orphans if we may have had a
                // power-loss, otherwise orphans are created intentionally
                // during operations such as lfs_mkdir
                if pass == 1 && tag == LfsError::NoEnt as i32 && powerloss {
                    // we are an orphan
                    LFS_DEBUG!("Fixing orphan {0:#x}, {0:#x}", pdir.tail[0], pdir.tail[1]);

                    // steal state
                    let err = lfs_dir_getgstate(lfs, &dir, &mut lfs.gdelta);
                    if err != 0 {
                        return err;
                    }

                    // steal tail
                    let mut tail = dir.tail;
                    lfs_pair_tole32(&mut tail);
                    let attrs = [
                        LfsMattr {
                            tag: LFS_MKTAG((LfsType::Tail as u16) + dir.split as u16, 0x3ff, 8),
                            buffer: tail.as_ptr() as *const c_void,
                        },
                    ];

                    let state = lfs_dir_orphaningcommit(lfs, &mut pdir, &attrs);
                    lfs_pair_fromle32(&mut tail);
                    if state < 0 {
                        return state;
                    }

                    // did our commit create more orphans?
                    if state == LfsOk::Orphaned as i32 {
                        moreorphans = true;
                    }

                    // refetch tail
                    continue;
                }
            }

            pdir = dir;
        }

        pass = if moreorphans { 0 } else { pass + 1 };
    }

    // mark orphans as fixed
    return lfs_fs_preporphans(lfs, -lfs_gstate_getorphans(&lfs.gstate));
}

/// 强制文件系统一致性
/// 
/// 依次执行超级块、移动和孤儿处理操作
fn lfs_fs_forceconsistency(lfs: &mut Lfs) -> i32 {
    let err = lfs_fs_desuperblock(lfs);
    if err != 0 {
        return err;
    }

    let err = lfs_fs_demove(lfs);
    if err != 0 {
        return err;
    }

    let err = lfs_fs_deorphan(lfs, true);
    if err != 0 {
        return err;
    }

    0
}

/// 使文件系统处于一致状态
/// lfs_fs_forceconsistency 完成大部分工作
fn lfs_fs_mkconsistent_(lfs: &mut Lfs) -> i32 {
    // lfs_fs_forceconsistency 完成大部分工作
    let err = lfs_fs_forceconsistency(lfs);
    if err != 0 {
        return err;
    }

    // 检查是否有任何待处理的 gstate
    let mut delta = LfsGstate { tag: 0, pair: [0, 0] };
    lfs_gstate_xor(&mut delta, &lfs.gdisk);
    lfs_gstate_xor(&mut delta, &lfs.gstate);
    if !lfs_gstate_iszero(&delta) {
        // lfs_dir_commit 将隐式写出任何待处理的 gstate
        let mut root = LfsMdir {
            pair: [0, 0],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0, 0],
        };
        
        let err = lfs_dir_fetch(lfs, &mut root, lfs.root);
        if err != 0 {
            return err;
        }

        let err = lfs_dir_commit(lfs, &mut root, core::ptr::null(), 0);
        if err != 0 {
            return err;
        }
    }

    0
}

/// 计算文件系统大小的回调函数
pub unsafe fn lfs_fs_size_count(p: *mut c_void, block: LfsBlock) -> i32 {
    let _ = block; // 忽略block参数
    let size = p as *mut LfsSize;
    *size += 1;
    0
}

/// 计算文件系统的总大小
fn lfs_fs_size_(lfs: *mut Lfs) -> LfsSsize {
    let mut size: LfsSize = 0;
    let err = unsafe { lfs_fs_traverse_(lfs, lfs_fs_size_count, &mut size as *mut _ as *mut c_void, false) };
    if err < 0 {
        return err;
    }

    size as LfsSsize
}

/// Force consistency and attempt to clean up garbage in the filesystem
fn lfs_fs_gc_(lfs: &mut Lfs) -> i32 {
    // force consistency, even if we're not necessarily going to write,
    // because this function is supposed to take care of janitorial work
    // isn't it?
    let err = lfs_fs_forceconsistency(lfs);
    if err != 0 {
        return err;
    }

    // try to compact metadata pairs, note we can't really accomplish
    // anything if compact_thresh doesn't at least leave a prog_size
    // available
    let cfg = unsafe { &*lfs.cfg };
    
    if cfg.compact_thresh < cfg.block_size - cfg.prog_size {
        // iterate over all mdirs
        let mut mdir = LfsMdir {
            pair: [0, 0],
            rev: 0,
            off: 0,
            etag: 0,
            count: 0,
            erased: false,
            split: false,
            tail: [0, 1],
        };
        
        while !lfs_pair_isnull(mdir.tail) {
            let err = lfs_dir_fetch(lfs, &mut mdir, mdir.tail);
            if err != 0 {
                return err;
            }

            // not erased? exceeds our compaction threshold?
            if !mdir.erased || (
                if cfg.compact_thresh == 0 {
                    mdir.off > cfg.block_size - cfg.block_size/8
                } else {
                    mdir.off > cfg.compact_thresh
                }) {
                // the easiest way to trigger a compaction is to mark
                // the mdir as unerased and add an empty commit
                mdir.erased = false;
                let err = lfs_dir_commit(lfs, &mut mdir, core::ptr::null(), 0);
                if err != 0 {
                    return err;
                }
            }
        }
    }

    // try to populate the lookahead buffer, unless it's already full
    if lfs.lookahead.size < 8 * unsafe { (*lfs.cfg).lookahead_size } {
        let err = lfs_alloc_scan(lfs);
        if err != 0 {
            return err;
        }
    }

    0
}

fn lfs_fs_grow_(lfs: &mut Lfs, block_count: LfsSize) -> i32 {
    // shrinking is not supported
    debug_assert!(block_count >= lfs.block_count);

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
            LFS_MKTAG(0x7ff, 0x3ff, 0),
            LFS_MKTAG(LfsType::InlineStruct as u16, 0, core::mem::size_of::<LfsSuperblock>() as u16),
            &mut superblock as *mut _ as *mut c_void
        );
        if tag < 0 {
            return tag;
        }
        lfs_superblock_fromle32(&mut superblock);

        superblock.block_count = lfs.block_count;

        lfs_superblock_tole32(&mut superblock);
        let err = lfs_dir_commit(
            lfs, 
            &mut root, 
            &[LfsMattr { 
                tag, 
                buffer: &superblock as *const _ as *const c_void 
            }]
        );
        if err != 0 {
            return err;
        }
    }

    0
}

fn lfs1_crc(crc: &mut u32, buffer: *const c_void, size: usize) {
    *crc = lfs_util::lfs_crc(*crc, buffer, size);
}

/// 从块设备读取数据
/// 
/// 如果我们曾经做过比交替对写入更多的操作，这可能需要考虑pcache
static unsafe fn lfs1_bd_read(lfs: *mut Lfs, block: LfsBlock,
        off: LfsOff, buffer: *mut c_void, size: LfsSize) -> i32 {
    // 调用底层块设备读取函数
    return lfs_bd_read(lfs, &(*lfs).pcache, &(*lfs).rcache, size,
            block, off, buffer, size);
}

/// 计算指定块区域的CRC校验和
///
/// 从块中读取数据并更新传入的CRC值
fn lfs1_bd_crc(lfs: &mut Lfs, block: LfsBlock, off: LfsOff, size: LfsSize, crc: &mut u32) -> i32 {
    for i in 0..size {
        let mut c: u8 = 0;
        let err = lfs1_bd_read(lfs, block, off + i, &mut c, 1);
        if err != 0 {
            return err;
        }

        lfs1_crc(crc, &c, 1);
    }

    0
}

/// 从小端格式转换lfs1磁盘目录结构数据为本地格式
fn lfs1_dir_fromle32(d: &mut crate::lfs1::LfsDiskDir) {
    d.rev = lfs_util::lfs_fromle32(d.rev);
    d.size = lfs_util::lfs_fromle32(d.size);
    d.tail[0] = lfs_util::lfs_fromle32(d.tail[0]);
    d.tail[1] = lfs_util::lfs_fromle32(d.tail[1]);
}

pub fn lfs1_dir_tole32(d: &mut lfs1_disk_dir) {
    d.rev = lfs_util::lfs_tole32(d.rev);
    d.size = lfs_util::lfs_tole32(d.size);
    d.tail[0] = lfs_util::lfs_tole32(d.tail[0]);
    d.tail[1] = lfs_util::lfs_tole32(d.tail[1]);
}

pub fn lfs1_entry_fromle32(d: &mut lfs1_disk_entry) {
    d.u.dir[0] = lfs_util::fromle32(d.u.dir[0]);
    d.u.dir[1] = lfs_util::fromle32(d.u.dir[1]);
}

/// 将LFS1磁盘条目中的目录字段转换为小端32位格式
pub fn lfs1_entry_tole32(d: &mut lfs1::LfsDiskEntry) {
    d.u.dir[0] = lfs_util::lfs_tole32(d.u.dir[0]);
    d.u.dir[1] = lfs_util::lfs_tole32(d.u.dir[1]);
}

/// Convert disk superblock fields from little-endian format
fn lfs1_superblock_fromle32(d: &mut lfs1_disk_superblock) {
    d.root[0] = lfs_util::fromle32(d.root[0]);
    d.root[1] = lfs_util::fromle32(d.root[1]);
    d.block_size = lfs_util::fromle32(d.block_size);
    d.block_count = lfs_util::fromle32(d.block_count);
    d.version = lfs_util::fromle32(d.version);
}

/// LFS1 entry size calculation
#[inline]
fn lfs1_entry_size(entry: &lfs1_entry_t) -> LfsSize {
    4 + entry.d.elen + entry.d.alen + entry.d.nlen
}

/// 从块中获取lfs1目录信息
fn lfs1_dir_fetch(lfs: &mut Lfs, dir: &mut Lfs1Dir, pair: &[LfsBlock; 2]) -> i32 {
    // 复制pair，否则可能会与dir引用冲突
    let tpair = [pair[0], pair[1]];
    let mut valid = false;

    // 检查两个块以获取最新修订版本
    for i in 0..2 {
        let mut test = Lfs1DiskDir::default();
        let err = lfs1_bd_read(lfs, tpair[i], 0, &mut test as *mut _ as *mut c_void, 
                               std::mem::size_of::<Lfs1DiskDir>() as u32);
        lfs1_dir_fromle32(&mut test);
        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                continue;
            }
            return err;
        }

        if valid && lfs_scmp(test.rev, dir.d.rev) < 0 {
            continue;
        }

        if (0x7fffffff & test.size) < (std::mem::size_of::<Lfs1DiskDir>() + 4) as u32 ||
           (0x7fffffff & test.size) > unsafe { (*lfs.cfg).block_size } {
            continue;
        }

        let mut crc: u32 = 0xffffffff;
        lfs1_dir_tole32(&mut test);
        lfs1_crc(&mut crc, &test as *const _ as *const c_void, 
                 std::mem::size_of::<Lfs1DiskDir>() as u32);
        lfs1_dir_fromle32(&mut test);
        let err = lfs1_bd_crc(lfs, tpair[i], std::mem::size_of::<Lfs1DiskDir>() as u32,
                             (0x7fffffff & test.size) - std::mem::size_of::<Lfs1DiskDir>() as u32, 
                             &mut crc);
        if err != 0 {
            if err == LfsError::Corrupt as i32 {
                continue;
            }
            return err;
        }

        if crc != 0 {
            continue;
        }

        valid = true;

        // 如果有效，设置dir
        dir.pair[0] = tpair[(i+0) % 2];
        dir.pair[1] = tpair[(i+1) % 2];
        dir.off = std::mem::size_of::<Lfs1DiskDir>() as u32;
        dir.d = test;
    }

    if !valid {
        LFS_ERROR!("Corrupted dir pair at {{0x{:x}, 0x{:x}}}", tpair[0], tpair[1]);
        return LfsError::Corrupt as i32;
    }

    0
}

/// 在LFS1目录中查找下一个条目
/// 
/// # Arguments
/// 
/// * `lfs` - 文件系统实例
/// * `dir` - 目录实例
/// * `entry` - 用于存储找到的条目信息
/// 
/// # Returns
/// 
/// * 成功时返回0
/// * 没有更多条目时返回LFS_ERR_NOENT
/// * 发生错误时返回负数错误码
fn lfs1_dir_next(lfs: *mut Lfs, dir: *mut crate::lfs1::Lfs1Dir, entry: *mut crate::lfs1::Lfs1Entry) -> i32 {
    unsafe {
        while (*dir).off + core::mem::size_of_val(&(*entry).d) > (0x7fffffff & (*dir).d.size) - 4 {
            if (0x80000000 & (*dir).d.size) == 0 {
                (*entry).off = (*dir).off;
                return LfsError::NoEnt as i32;
            }

            let err = crate::lfs1::lfs1_dir_fetch(lfs, dir, (*dir).d.tail);
            if err != 0 {
                return err;
            }

            (*dir).off = core::mem::size_of_val(&(*dir).d) as u32;
            (*dir).pos += (core::mem::size_of_val(&(*dir).d) + 4) as u32;
        }

        let err = crate::lfs1::lfs1_bd_read(
            lfs,
            (*dir).pair[0],
            (*dir).off,
            &mut (*entry).d as *mut _ as *mut c_void,
            core::mem::size_of_val(&(*entry).d) as u32,
        );
        
        crate::lfs1::lfs1_entry_fromle32(&mut (*entry).d);
        
        if err != 0 {
            return err;
        }

        (*entry).off = (*dir).off;
        (*dir).off += crate::lfs1::lfs1_entry_size(entry);
        (*dir).pos += crate::lfs1::lfs1_entry_size(entry);
        
        0
    }
}

/// 检查LFS1实体是否已移动
fn lfs1_moved(lfs: &mut Lfs, e: *const c_void) -> i32 {
    if lfs_pair_isnull(unsafe { (*lfs.lfs1).root }) {
        return 0;
    }

    // 跳过超级块
    let mut cwd = lfs1_dir_t::default();
    let err = lfs1_dir_fetch(lfs, &mut cwd, &[0, 1]);
    if err != 0 {
        return err;
    }

    // 遍历所有目录条目
    let mut entry = lfs1_entry_t::default();
    while !lfs_pair_isnull(cwd.d.tail) {
        let err = lfs1_dir_fetch(lfs, &mut cwd, cwd.d.tail);
        if err != 0 {
            return err;
        }

        loop {
            let err = lfs1_dir_next(lfs, &mut cwd, &mut entry);
            if err != 0 && err != LfsError::NoEnt as i32 {
                return err;
            }

            if err == LfsError::NoEnt as i32 {
                break;
            }

            if (0x80 & entry.d.type_) == 0 &&
               unsafe { libc::memcmp(&entry.d.u as *const _ as *const c_void, e, core::mem::size_of_val(&entry.d.u)) } == 0 {
                return 1; // true
            }
        }
    }

    return 0; // false
}

fn lfs1_mount(lfs: &mut Lfs, lfs1: *mut c_void, cfg: &LfsConfig) -> i32 {
    let mut err = 0;
    {
        err = lfs_init(lfs, cfg);
        if err != 0 {
            return err;
        }

        lfs.lfs1 = lfs1;
        // 安全地获取lfs1指针并设置root值
        unsafe {
            // 这里假设lfs1有root字段是一个[LfsBlock; 2]数组
            let lfs1_struct = &mut *(lfs1 as *mut crate::lfs1::Lfs1);
            lfs1_struct.root[0] = LFS_BLOCK_NULL;
            lfs1_struct.root[1] = LFS_BLOCK_NULL;
        }

        // 设置前瞻缓冲区
        lfs.lookahead.start = 0;
        lfs.lookahead.size = 0;
        lfs.lookahead.next = 0;
        lfs_alloc_ckpoint(lfs);

        // 加载超级块
        let mut dir = crate::lfs1::Lfs1Dir::default();
        let mut superblock = crate::lfs1::Lfs1Superblock::default();
        
        let root_pair = [0, 1];
        err = lfs1_dir_fetch(lfs, &mut dir, &root_pair);
        if err != 0 && err != LfsError::Corrupt as i32 {
            goto_cleanup(lfs);
            return err;
        }

        if err == 0 {
            err = lfs1_bd_read(
                lfs,
                dir.pair[0],
                core::mem::size_of_val(&dir.d) as u32,
                &mut superblock.d as *mut _ as *mut c_void,
                core::mem::size_of_val(&superblock.d) as u32
            );
            lfs1_superblock_fromle32(&mut superblock.d);
            if err != 0 {
                goto_cleanup(lfs);
                return err;
            }

            // 安全地获取lfs1指针并设置root值
            unsafe {
                let lfs1_struct = &mut *(lfs1 as *mut crate::lfs1::Lfs1);
                lfs1_struct.root[0] = superblock.d.root[0];
                lfs1_struct.root[1] = superblock.d.root[1];
            }
        }

        // 检查超级块有效性
        let magic_matches = unsafe {
            let magic = superblock.d.magic.as_ptr();
            let littlefs_str = "littlefs".as_bytes().as_ptr();
            lfs_util::memcmp(
                magic as *const c_void,
                littlefs_str as *const c_void,
                8
            ) == 0
        };

        if err != 0 || !magic_matches {
            lfs_error!(
                "Invalid superblock at {{0x{:x}, 0x{:x}}}",
                0, 1
            );
            err = LfsError::Corrupt as i32;
            goto_cleanup(lfs);
            return err;
        }

        let major_version = 0xffff & (superblock.d.version >> 16);
        let minor_version = 0xffff & (superblock.d.version >> 0);
        if major_version != crate::lfs1::LFS1_DISK_VERSION_MAJOR ||
           minor_version > crate::lfs1::LFS1_DISK_VERSION_MINOR {
            lfs_error!(
                "Invalid version v{}.{}",
                major_version, minor_version
            );
            err = LfsError::Inval as i32;
            goto_cleanup(lfs);
            return err;
        }

        return 0;
    }

    fn goto_cleanup(lfs: &mut Lfs) {
        lfs_deinit(lfs);
    }
}

/// 卸载文件系统，释放所有内部资源
pub unsafe fn lfs1_unmount(lfs: *mut Lfs) -> i32 {
    lfs_deinit(lfs)
}

fn lfs_migrate_(lfs: &mut Lfs, cfg: &LfsConfig) -> i32 {
    let mut lfs1 = unsafe { std::mem::zeroed::<crate::lfs1::Lfs1>() };

    // Indeterminate filesystem size not allowed for migration.
    assert!(cfg.block_count != 0);

    let err = unsafe { crate::lfs1::lfs1_mount(lfs, &mut lfs1, cfg) };
    if err != 0 {
        return err;
    }

    {
        // iterate through each directory, copying over entries
        // into new directory
        let mut dir1 = unsafe { std::mem::zeroed::<crate::lfs1::Lfs1Dir>() };
        let mut dir2 = unsafe { std::mem::zeroed::<LfsMdir>() };
        
        let lfs1_ptr = lfs.lfs1 as *mut crate::lfs1::Lfs1;
        unsafe {
            dir1.d.tail[0] = (*lfs1_ptr).root[0];
            dir1.d.tail[1] = (*lfs1_ptr).root[1];
        }
        
        while !lfs_pair_isnull(&dir1.d.tail) {
            // iterate old dir
            let err = unsafe { crate::lfs1::lfs1_dir_fetch(lfs, &mut dir1, &dir1.d.tail) };
            if err != 0 {
                goto_cleanup!(err);
            }

            // create new dir and bind as temporary pretend root
            let err = lfs_dir_alloc(lfs, &mut dir2);
            if err != 0 {
                goto_cleanup!(err);
            }

            dir2.rev = dir1.d.rev;
            dir1.head[0] = dir1.pair[0];
            dir1.head[1] = dir1.pair[1];
            lfs.root[0] = dir2.pair[0];
            lfs.root[1] = dir2.pair[1];

            let err = lfs_dir_commit(lfs, &mut dir2, core::ptr::null(), 0);
            if err != 0 {
                goto_cleanup!(err);
            }

            loop {
                let mut entry1 = unsafe { std::mem::zeroed::<crate::lfs1::Lfs1Entry>() };
                let err = unsafe { crate::lfs1::lfs1_dir_next(lfs, &mut dir1, &mut entry1) };
                if err != 0 && err != LfsError::NoEnt as i32 {
                    goto_cleanup!(err);
                }

                if err == LfsError::NoEnt as i32 {
                    break;
                }

                // check that entry has not been moved
                if entry1.d.type_ & 0x80 != 0 {
                    let moved = unsafe { crate::lfs1::lfs1_moved(lfs, &entry1.d.u) };
                    if moved < 0 {
                        goto_cleanup!(moved);
                    }

                    if moved != 0 {
                        continue;
                    }

                    entry1.d.type_ &= !0x80;
                }

                // also fetch name
                let mut name = [0u8; LFS_NAME_MAX+1];
                let err = unsafe {
                    crate::lfs1::lfs1_bd_read(
                        lfs, 
                        dir1.pair[0],
                        entry1.off + 4 + entry1.d.elen + entry1.d.alen,
                        name.as_mut_ptr() as *mut c_void, 
                        entry1.d.nlen
                    )
                };
                if err != 0 {
                    goto_cleanup!(err);
                }

                let isdir = entry1.d.type_ == crate::lfs1::Lfs1Type::Dir as u8;

                // create entry in new dir
                let err = lfs_dir_fetch(lfs, &mut dir2, &lfs.root);
                if err != 0 {
                    goto_cleanup!(err);
                }

                let mut id: u16 = 0;
                let name_ptr = name.as_ptr() as *const i8;
                let err = lfs_dir_find(lfs, &mut dir2, &name_ptr, &mut id);
                if !(err == LfsError::NoEnt as i32 && id != 0x3ff) {
                    goto_cleanup!(if err < 0 { err } else { LfsError::Exist as i32 });
                }

                unsafe { crate::lfs1::lfs1_entry_tole32(&mut entry1.d) };
                
                let attrs = [
                    LfsMattr { 
                        tag: LFS_MKTAG!(LfsType::Create as u16, id, 0), 
                        buffer: core::ptr::null() 
                    },
                    LfsMattr { 
                        tag: if isdir {
                            LFS_MKTAG!(LfsType::Dir as u16, id, entry1.d.nlen as usize)
                        } else {
                            LFS_MKTAG!(LfsType::Reg as u16, id, entry1.d.nlen as usize)
                        },
                        buffer: name.as_ptr() as *const c_void 
                    },
                    LfsMattr { 
                        tag: if isdir {
                            LFS_MKTAG!(LfsType::DirStruct as u16, id, core::mem::size_of::<crate::lfs1::Lfs1Ctz>())
                        } else {
                            LFS_MKTAG!(LfsType::CtzStruct as u16, id, core::mem::size_of::<crate::lfs1::Lfs1Ctz>())
                        },
                        buffer: &entry1.d.u as *const _ as *const c_void 
                    }
                ];
                
                let err = lfs_dir_commit(lfs, &mut dir2, attrs.as_ptr(), attrs.len() as i32);
                unsafe { crate::lfs1::lfs1_entry_fromle32(&mut entry1.d) };
                if err != 0 {
                    goto_cleanup!(err);
                }
            }

            if !lfs_pair_isnull(&dir1.d.tail) {
                // find last block and update tail to thread into fs
                let err = lfs_dir_fetch(lfs, &mut dir2, &lfs.root);
                if err != 0 {
                    goto_cleanup!(err);
                }

                while dir2.split {
                    let err = lfs_dir_fetch(lfs, &mut dir2, &dir2.tail);
                    if err != 0 {
                        goto_cleanup!(err);
                    }
                }

                lfs_pair_tole32(&mut dir2.pair);
                let attrs = [LfsMattr {
                    tag: LFS_MKTAG!(LfsType::SoftTail as u16, 0x3ff, 8),
                    buffer: dir1.d.tail.as_ptr() as *const c_void
                }];
                let err = lfs_dir_commit(lfs, &mut dir2, attrs.as_ptr(), 1);
                lfs_pair_fromle32(&mut dir2.pair);
                if err != 0 {
                    goto_cleanup!(err);
                }
            }

            // Copy over first block to thread into fs. Unfortunately
            // if this fails there is not much we can do.
            debug!(
                "Migrating {{0x{:08x}, 0x{:08x}}} -> {{0x{:08x}, 0x{:08x}}}",
                lfs.root[0], lfs.root[1], dir1.head[0], dir1.head[1]
            );

            let err = lfs_bd_erase(lfs, dir1.head[1]);
            if err != 0 {
                goto_cleanup!(err);
            }

            let err = lfs_dir_fetch(lfs, &mut dir2, &lfs.root);
            if err != 0 {
                goto_cleanup!(err);
            }

            for i in 0..dir2.off {
                let mut dat = 0u8;
                let err = lfs_bd_read(
                    lfs,
                    core::ptr::null(),
                    &lfs.rcache,
                    dir2.off,
                    dir2.pair[0],
                    i,
                    &mut dat as *mut u8 as *mut c_void,
                    1
                );
                if err != 0 {
                    goto_cleanup!(err);
                }

                let err = lfs_bd_prog(
                    lfs,
                    &lfs.pcache,
                    &lfs.rcache,
                    true,
                    dir1.head[1],
                    i,
                    &dat as *const u8 as *const c_void,
                    1
                );
                if err != 0 {
                    goto_cleanup!(err);
                }
            }

            let err = lfs_bd_flush(lfs, &lfs.pcache, &lfs.rcache, true);
            if err != 0 {
                goto_cleanup!(err);
            }
        }

        // Create new superblock. This marks a successful migration!
        let err = unsafe { crate::lfs1::lfs1_dir_fetch(lfs, &mut dir1, &[0, 1]) };
        if err != 0 {
            goto_cleanup!(err);
        }

        dir2.pair[0] = dir1.pair[0];
        dir2.pair[1] = dir1.pair[1];
        dir2.rev = dir1.d.rev;
        dir2.off = core::mem::size_of_val(&dir2.rev) as LfsOff;
        dir2.etag = 0xffffffff;
        dir2.count = 0;
        let lfs1_ptr = lfs.lfs1 as *mut crate::lfs1::Lfs1;
        unsafe {
            dir2.tail[0] = (*lfs1_ptr).root[0];
            dir2.tail[1] = (*lfs1_ptr).root[1];
        }
        dir2.erased = false;
        dir2.split = true;

        let mut superblock = LfsSuperblock {
            version: LFS_DISK_VERSION,
            block_size: unsafe { (*lfs.cfg).block_size },
            block_count: unsafe { (*lfs.cfg).block_count },
            name_max: lfs.name_max,
            file_max: lfs.file_max,
            attr_max: lfs.attr_max,
        };

        lfs_superblock_tole32(&mut superblock);
        
        let attrs = [
            LfsMattr {
                tag: LFS_MKTAG!(LfsType::Create as u16, 0, 0),
                buffer: core::ptr::null()
            },
            LfsMattr {
                tag: LFS_MKTAG!(LfsType::SuperBlock as u16, 0, 8),
                buffer: "littlefs".as_ptr() as *const c_void
            },
            LfsMattr {
                tag: LFS_MKTAG!(LfsType::InlineStruct as u16, 0, core::mem::size_of::<LfsSuperblock>()),
                buffer: &superblock as *const _ as *const c_void
            }
        ];
        
        let err = lfs_dir_commit(lfs, &mut dir2, attrs.as_ptr(), attrs.len() as i32);
        if err != 0 {
            goto_cleanup!(err);
        }

        // sanity check that fetch works
        let err = lfs_dir_fetch(lfs, &mut dir2, &[0, 1]);
        if err != 0 {
            goto_cleanup!(err);
        }

        // force compaction to prevent accidentally mounting v1
        dir2.erased = false;
        let err = lfs_dir_commit(lfs, &mut dir2, core::ptr::null(), 0);
        if err != 0 {
            goto_cleanup!(err);
        }
    }

    let err = unsafe { crate::lfs1::lfs1_unmount(lfs) };
    return err;

    // Cleanup handler for early returns
    unsafe fn goto_cleanup(err: i32) -> i32 {
        let lfs = std::mem::zeroed::<*mut Lfs>();
        crate::lfs1::lfs1_unmount(lfs);
        err
    }
}

pub fn lfs_format(lfs: *mut Lfs, cfg: *const LfsConfig) -> i32 {
    let err = unsafe { LFS_LOCK(cfg) };
    if err != 0 {
        return err;
    }

    LFS_TRACE!("lfs_format({:p}, {:p} {{.context={:p}, \
                 .read={:p}, .prog={:p}, .erase={:p}, .sync={:p}, \
                 .read_size={}, .prog_size={}, \
                 .block_size={}, .block_count={}, \
                 .block_cycles={}, .cache_size={}, \
                 .lookahead_size={}, .read_buffer={:p}, \
                 .prog_buffer={:p}, .lookahead_buffer={:p}, \
                 .name_max={}, .file_max={}, \
                 .attr_max={:?}}})",
             lfs, cfg, unsafe { (*cfg).context },
             unsafe { (*cfg).read.map_or(core::ptr::null(), |f| f as *const _) },
             unsafe { (*cfg).prog.map_or(core::ptr::null(), |f| f as *const _) },
             unsafe { (*cfg).erase.map_or(core::ptr::null(), |f| f as *const _) },
             unsafe { (*cfg).sync.map_or(core::ptr::null(), |f| f as *const _) },
             unsafe { (*cfg).read_size }, unsafe { (*cfg).prog_size },
             unsafe { (*cfg).block_size }, unsafe { (*cfg).block_count },
             unsafe { (*cfg).block_cycles }, unsafe { (*cfg).cache_size },
             unsafe { (*cfg).lookahead_size }, unsafe { (*cfg).read_buffer },
             unsafe { (*cfg).prog_buffer }, unsafe { (*cfg).lookahead_buffer },
             unsafe { (*cfg).name_max }, unsafe { (*cfg).file_max },
             unsafe { (*cfg).attr_max });

    let err = lfs_format_(lfs, cfg);

    LFS_TRACE!("lfs_format -> {}", err);
    unsafe { LFS_UNLOCK(cfg) };
    err
}

/// Mount a littlefs file system
pub fn lfs_mount(lfs: *mut Lfs, cfg: *const LfsConfig) -> i32 {
    let err = if let Some(lock_fn) = unsafe { (*cfg).lock } {
        unsafe { lock_fn(cfg) }
    } else {
        0
    };

    if err != 0 {
        return err;
    }

    lfs_util::trace!("lfs_mount({:p}, {:p} {{.context={:p}, \
                .read={:p}, .prog={:p}, .erase={:p}, .sync={:p}, \
                .read_size={}, .prog_size={}, \
                .block_size={}, .block_count={}, \
                .block_cycles={}, .cache_size={}, \
                .lookahead_size={}, .read_buffer={:p}, \
                .prog_buffer={:p}, .lookahead_buffer={:p}, \
                .name_max={}, .file_max={}, \
                .attr_max={}}})",
            lfs, cfg, unsafe { (*cfg).context },
            unsafe { (*cfg).read.map_or(core::ptr::null(), |f| f as *const ()) },
            unsafe { (*cfg).prog.map_or(core::ptr::null(), |f| f as *const ()) },
            unsafe { (*cfg).erase.map_or(core::ptr::null(), |f| f as *const ()) },
            unsafe { (*cfg).sync.map_or(core::ptr::null(), |f| f as *const ()) },
            unsafe { (*cfg).read_size }, unsafe { (*cfg).prog_size },
            unsafe { (*cfg).block_size }, unsafe { (*cfg).block_count },
            unsafe { (*cfg).block_cycles }, unsafe { (*cfg).cache_size },
            unsafe { (*cfg).lookahead_size }, unsafe { (*cfg).read_buffer },
            unsafe { (*cfg).prog_buffer }, unsafe { (*cfg).lookahead_buffer },
            unsafe { (*cfg).name_max }, unsafe { (*cfg).file_max },
            unsafe { (*cfg).attr_max });

    let err = lfs_mount_(lfs, cfg);

    lfs_util::trace!("lfs_mount -> {}", err);
    
    if let Some(unlock_fn) = unsafe { (*cfg).unlock } {
        unsafe { unlock_fn(cfg) };
    }
    
    err
}

/// Unmount a littlefs instance
pub fn lfs_unmount(lfs: *mut Lfs) -> i32 {
    let err = unsafe { LFS_LOCK((*lfs).cfg) };
    if err != 0 {
        return err;
    }
    unsafe { lfs_util::trace!("lfs_unmount(%p)", lfs as *mut c_void) };

    let err = unsafe { lfs_unmount_(lfs) };

    unsafe { lfs_util::trace!("lfs_unmount -> %d", err) };
    unsafe { LFS_UNLOCK((*lfs).cfg) };
    err
}

/// Remove a file or directory
pub fn lfs_remove(lfs: *mut Lfs, path: *const u8) -> i32 {
    let err = unsafe {
        if let Some(lock_fn) = (*(*lfs).cfg).lock {
            lock_fn((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_remove({:p}, {:?})", lfs, path);

    let err = unsafe { lfs_remove_(lfs, path) };

    lfs_util::trace!("lfs_remove -> {}", err);
    
    unsafe {
        if let Some(unlock_fn) = (*(*lfs).cfg).unlock {
            unlock_fn((*lfs).cfg);
        }
    }
    
    err
}

/// 重命名文件
pub fn lfs_rename(lfs: *mut Lfs, oldpath: *const i8, newpath: *const i8) -> i32 {
    let err = unsafe { 
        if let Some(lock) = (*(*lfs).cfg).lock {
            lock((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_rename({:p}, \"{}\", \"{}\")", lfs, 
        unsafe { lfs_util::cstr_to_str(oldpath) }, 
        unsafe { lfs_util::cstr_to_str(newpath) });

    let err = unsafe { lfs_rename_(lfs, oldpath, newpath) };

    lfs_util::trace!("lfs_rename -> {}", err);
    
    unsafe {
        if let Some(unlock) = (*(*lfs).cfg).unlock {
            unlock((*lfs).cfg);
        }
    }
    
    err
}

pub fn lfs_stat(lfs: *mut Lfs, path: *const u8, info: *mut LfsInfo) -> i32 {
    let err = unsafe { LFS_LOCK((*lfs).cfg) };
    if err != 0 {
        return err;
    }
    
    unsafe { lfs_util::trace!("lfs_stat({:p}, \"{}\", {:p})", lfs, path, info) };

    let err = unsafe { lfs_stat_(lfs, path, info) };

    unsafe { lfs_util::trace!("lfs_stat -> {}", err) };
    unsafe { LFS_UNLOCK((*lfs).cfg) };
    err
}

/// 获取文件或目录的自定义属性
///
/// 从文件或目录中读取自定义属性。
///
/// 返回实际读取的属性大小，如果发生错误则返回负的错误代码。
/// 与属性相关的可能错误码：
/// - LfsError::NoAttr - 找不到请求的属性
pub fn lfs_getattr(lfs: *mut Lfs, path: *const u8, type_: u8, buffer: *mut c_void, size: LfsSize) -> LfsSsize {
    let err = unsafe { 
        if let Some(lock) = (*(*lfs).cfg).lock {
            lock((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    #[cfg(feature = "trace")]
    unsafe {
        lfs_util::trace!("lfs_getattr(%p, \"%s\", %u, %p, %u)", 
                        lfs, path, type_, buffer, size);
    }

    let res = unsafe { lfs_getattr_(lfs, path, type_, buffer, size) };

    #[cfg(feature = "trace")]
    unsafe {
        lfs_util::trace!("lfs_getattr -> %d", res);
    }
    
    unsafe {
        if let Some(unlock) = (*(*lfs).cfg).unlock {
            unlock((*lfs).cfg);
        }
    }
    
    res
}

/// 设置文件属性
/// 
/// 在路径对应的文件或目录上设置自定义属性。
/// 这些属性被标记类型索引，同一类型的属性将被覆盖。
/// 
/// 注意：类似于lfs_file_opencfg中的attr参数，
/// 自定义属性的大小被限制为lfs->attr_max
/// (传递给lfs_mount的attr_max配置参数)。
///
/// 成功时返回0，失败时返回负的错误代码。
pub unsafe fn lfs_setattr(lfs: *mut Lfs, path: *const u8, 
                         type_: u8, buffer: *const c_void, size: LfsSize) -> i32 {
    let err = LFS_LOCK((*lfs).cfg);
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_setattr(%p, \"%s\", %u, %p, %u)",
            lfs, path, type_, buffer, size);

    let err = lfs_setattr_(lfs, path, type_, buffer, size);

    lfs_util::trace!("lfs_setattr -> %d", err);
    LFS_UNLOCK((*lfs).cfg);
    return err;
}

/// 从文件中移除属性
pub unsafe fn lfs_removeattr(lfs: *mut Lfs, path: *const u8, type_: u8) -> i32 {
    let err = if let Some(lock_fn) = (*(*lfs).cfg).lock {
        lock_fn((*lfs).cfg)
    } else {
        0
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_removeattr({:p}, \"{}\", {})", 
        lfs, 
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(
            path, 
            libc::strlen(path as *const i8)
        )), 
        type_);

    let err = lfs_removeattr_(lfs, path, type_);

    lfs_util::trace!("lfs_removeattr -> {}", err);
    
    if let Some(unlock_fn) = (*(*lfs).cfg).unlock {
        unlock_fn((*lfs).cfg);
    }
    
    err
}

pub fn lfs_file_open(lfs: *mut Lfs, file: *mut LfsFile, path: *const i8, flags: i32) -> i32 {
    let err = LFS_LOCK((*lfs).cfg);
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_file_open(%p, %p, \"%s\", %x)",
            lfs as *mut c_void, file as *mut c_void, path, flags as u32);
    
    debug_assert!(!lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

    let err = lfs_file_open_(lfs, file, path, flags);

    lfs_util::trace!("lfs_file_open -> %d", err);
    LFS_UNLOCK((*lfs).cfg);
    err
}

pub fn lfs_file_opencfg(lfs: *mut Lfs, file: *mut LfsFile, path: *const u8, flags: i32, cfg: *const LfsFileConfig) -> i32 {
    let err = unsafe { 
        if let Some(lock) = (*(*lfs).cfg).lock {
            lock((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    unsafe {
        lfs_util::trace!("lfs_file_opencfg({:p}, {:p}, \"{}\", {:#x}, {:p} {{.buffer={:p}, .attrs={:p}, .attr_count={}}})",
            lfs, file, core::str::from_utf8_unchecked(core::slice::from_raw_parts(path, libc::strlen(path as *const libc::c_char))),
            flags as u32, cfg, (*cfg).buffer, (*cfg).attrs, (*cfg).attr_count);
        
        // 确保文件没有被打开
        debug_assert!(!lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));
        
        // 调用内部实现函数
        let err = lfs_file_opencfg_(lfs, file, path, flags, cfg);
        
        lfs_util::trace!("lfs_file_opencfg -> {}", err);
        
        // 解锁
        if let Some(unlock) = (*(*lfs).cfg).unlock {
            unlock((*lfs).cfg);
        }
        
        err
    }
}

/// 关闭一个已打开的文件
///
/// 任何挂起的写入操作将被刷新到磁盘，并且分配的资源将被释放
pub unsafe fn lfs_file_close(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    let err = LFS_LOCK((*lfs).cfg);
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_file_close(%p, %p)", lfs, file);
    lfs_util::assert!(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

    let err = lfs_file_close_(lfs, file);

    lfs_util::trace!("lfs_file_close -> {}", err);
    LFS_UNLOCK((*lfs).cfg);
    err
}

/// 同步文件，将所有缓冲数据写入磁盘
pub fn lfs_file_sync(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    // 尝试获取锁
    let err = unsafe { LFS_LOCK((*lfs).cfg) };
    if err != 0 {
        return err;
    }
    
    // 跟踪调用
    lfs_util::LFS_TRACE("lfs_file_sync(%p, %p)", lfs as *mut c_void, file as *mut c_void);
    
    // 断言文件在挂载列表中是打开的
    debug_assert!(unsafe { lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist) });

    // 执行实际的同步操作
    let err = unsafe { lfs_file_sync_(lfs, file) };

    // 跟踪返回结果
    lfs_util::LFS_TRACE("lfs_file_sync -> %d", err);
    
    // 释放锁
    unsafe { LFS_UNLOCK((*lfs).cfg) };
    
    err
}

/// 从文件中读取数据
///
/// 从文件中的当前位置读取数据到提供的缓冲区中。
/// 当前文件位置会随着读取的字节数更新。
///
/// 返回读取的字节数，或者在错误时返回负错误代码。
pub fn lfs_file_read(lfs: *mut Lfs, file: *mut LfsFile, buffer: *mut c_void, size: LfsSize) -> LfsSsize {
    unsafe {
        // 获取锁
        let err = lfs_util::LFS_LOCK((*lfs).cfg);
        if err != 0 {
            return err;
        }
        
        // 调试日志
        lfs_util::LFS_TRACE(
            "lfs_file_read(%p, %p, %p, %"PRIu32")",
            lfs as *mut c_void,
            file as *mut c_void,
            buffer,
            size
        );
        
        // 确认文件已打开
        lfs_util::LFS_ASSERT(lfs_util::lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

        // 调用实际的读取实现
        let res = lfs_util::lfs_file_read_(lfs, file, buffer, size);

        // 调试日志
        lfs_util::LFS_TRACE("lfs_file_read -> %"PRId32, res);
        
        // 释放锁
        lfs_util::LFS_UNLOCK((*lfs).cfg);
        
        res
    }
}

/// Write data to file
///
/// Note that file's position is updated by this. Returns the number of bytes 
/// written, or a negative error code on failure.
pub fn lfs_file_write(lfs: *mut Lfs, file: *mut LfsFile, buffer: *const c_void, size: LfsSize) -> LfsSsize {
    // Acquire lock
    let mut err = match unsafe { (*lfs).cfg.as_ref().unwrap().lock.unwrap()((*lfs).cfg) } {
        0 => 0,
        e => return e
    };
    
    // Trace operation
    lfs_util::trace!(
        "lfs_file_write({:p}, {:p}, {:p}, {})",
        lfs,
        file,
        buffer,
        size
    );
    
    // Assert that file is open
    debug_assert!(lfs_util::lfs_mlist_isopen(unsafe { (*lfs).mlist }, file as *mut LfsMlist));

    // Perform the write operation
    let res = lfs_util::lfs_file_write_(lfs, file, buffer, size);

    // Trace result
    lfs_util::trace!("lfs_file_write -> {}", res);
    
    // Release lock
    unsafe { (*lfs).cfg.as_ref().unwrap().unlock.unwrap()((*lfs).cfg) };
    
    res
}

/// Move the position of the file to the given offset
/// 
/// Returns the new position of the file, or a negative error code
pub fn lfs_file_seek(lfs: *mut Lfs, file: *mut LfsFile, off: LfsSoff, whence: i32) -> LfsSoff {
    let err = if let Some(lock_fn) = unsafe { (*lfs).cfg.as_ref().unwrap().lock } {
        unsafe { lock_fn((*lfs).cfg) }
    } else {
        0
    };
    
    if err != 0 {
        return err;
    }

    lfs_util::trace!("lfs_file_seek({:p}, {:p}, {}, {})",
        lfs, file, off, whence);
    lfs_util::assert!(lfs_mlist_isopen(unsafe { (*lfs).mlist }, file as *mut LfsMlist));

    let res = lfs_file_seek_(lfs, file, off, whence);

    lfs_util::trace!("lfs_file_seek -> {}", res);
    
    if let Some(unlock_fn) = unsafe { (*lfs).cfg.as_ref().unwrap().unlock } {
        unsafe { unlock_fn((*lfs).cfg) };
    }
    
    res
}

/// 截断文件到指定大小
pub fn lfs_file_truncate(lfs: *mut Lfs, file: *mut LfsFile, size: LfsOff) -> i32 {
    unsafe {
        // 锁定文件系统
        let err = LFS_LOCK((*lfs).cfg);
        if err != 0 {
            return err;
        }

        // 记录操作
        LFS_TRACE("lfs_file_truncate(%p, %p, %u)",
                  lfs as *mut c_void, file as *mut c_void, size);
        
        // 确保文件是打开的
        LFS_ASSERT(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

        // 调用内部实现
        let err = lfs_file_truncate_(lfs, file, size);

        // 记录结果
        LFS_TRACE("lfs_file_truncate -> %d", err);
        
        // 解锁文件系统
        LFS_UNLOCK((*lfs).cfg);
        
        err
    }
}

pub unsafe fn lfs_file_tell(lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    // Lock and check if file is valid
    let err = LFS_LOCK((*lfs).cfg);
    if err != 0 {
        return err;
    }
    LFS_TRACE("lfs_file_tell(%p, %p)", lfs as *const c_void, file as *const c_void);
    LFS_ASSERT(lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist));

    // Get the position
    let res = lfs_file_tell_(lfs, file);

    // Unlock and return
    LFS_TRACE("lfs_file_tell -> %d", res);
    LFS_UNLOCK((*lfs).cfg);
    res
}

// 将文件位置重置为开头
pub fn lfs_file_rewind(lfs: *mut Lfs, file: *mut LfsFile) -> i32 {
    unsafe {
        let cfg = (*lfs).cfg;
        let err = LFS_LOCK(cfg);
        if err != 0 {
            return err;
        }
        
        lfs_util::trace!("lfs_file_rewind({:p}, {:p})", lfs, file);

        let err = lfs_file_rewind_(lfs, file);

        lfs_util::trace!("lfs_file_rewind -> {}", err);
        LFS_UNLOCK(cfg);
        err
    }
}

/// Returns the size of the file in bytes
pub fn lfs_file_size(lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    // 获取锁
    let err = unsafe { 
        if let Some(lock) = (*(*lfs).cfg).lock {
            lock((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    #[cfg(feature = "trace")]
    log::trace!("lfs_file_size({:p}, {:p})", lfs, file);
    
    // 验证文件是否打开
    debug_assert!(unsafe { lfs_mlist_isopen((*lfs).mlist, file as *mut LfsMlist) });

    // 调用内部尺寸函数
    let res = unsafe { lfs_file_size_(lfs, file) };

    #[cfg(feature = "trace")]
    log::trace!("lfs_file_size -> {}", res);
    
    // 释放锁
    unsafe {
        if let Some(unlock) = (*(*lfs).cfg).unlock {
            unlock((*lfs).cfg);
        }
    }
    
    res
}

pub fn lfs_mkdir(lfs: *mut Lfs, path: *const u8) -> i32 {
    let err = unsafe {
        if let Some(lock_fn) = (*(*lfs).cfg).lock {
            lock_fn((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::LFS_TRACE("lfs_mkdir(%p, \"%s\")", lfs as *mut c_void, path);

    let err = unsafe { lfs_mkdir_(lfs, path) };

    lfs_util::LFS_TRACE("lfs_mkdir -> %d", err);
    
    unsafe {
        if let Some(unlock_fn) = (*(*lfs).cfg).unlock {
            unlock_fn((*lfs).cfg);
        }
    }
    
    err
}

/// 打开目录。
/// 
/// 注意: 禁止打开已打开的目录
pub fn lfs_dir_open(lfs: *mut Lfs, dir: *mut LfsDir, path: *const u8) -> i32 {
    // 调用锁定函数
    let err = unsafe {
        if let Some(lock_fn) = (*(*lfs).cfg).lock {
            lock_fn((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    // 跟踪调用
    lfs_util::trace!("lfs_dir_open({:p}, {:p}, {:?})", lfs, dir, path);
    
    // 断言目录不在挂载列表中
    debug_assert!(!unsafe { lfs_mlist_isopen((*lfs).mlist, dir as *mut LfsMlist) });
    
    // 调用内部目录打开函数
    let err = unsafe { lfs_dir_open_(lfs, dir, path) };
    
    // 跟踪返回结果
    lfs_util::trace!("lfs_dir_open -> {}", err);
    
    // 解锁
    unsafe {
        if let Some(unlock_fn) = (*(*lfs).cfg).unlock {
            unlock_fn((*lfs).cfg);
        }
    }
    
    err
}

/// 关闭目录
pub fn lfs_dir_close(lfs: *mut Lfs, dir: *mut LfsDir) -> i32 {
    // 尝试锁定文件系统
    let err = unsafe { 
        if let Some(lock) = (*(*lfs).cfg).lock {
            lock((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }

    unsafe {
        lfs_util::trace!("lfs_dir_close(%p, %p)", lfs, dir);

        // 调用内部目录关闭函数
        let err = lfs_dir_close_(lfs, dir);

        lfs_util::trace!("lfs_dir_close -> {}", err);
        
        // 解锁文件系统
        if let Some(unlock) = (*(*lfs).cfg).unlock {
            unlock((*lfs).cfg);
        }
        
        err
    }
}

/// 读取目录条目信息
///
/// 从打开的目录中读取下一个目录条目信息到提供的info结构中
pub fn lfs_dir_read(lfs: *mut Lfs, dir: *mut LfsDir, info: *mut LfsInfo) -> i32 {
    let cfg = unsafe { (*lfs).cfg };
    let err = unsafe { 
        if let Some(lock_fn) = (*cfg).lock {
            lock_fn(cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_dir_read({:p}, {:p}, {:p})", 
                    lfs as *const c_void,
                    dir as *const c_void, 
                    info as *const c_void);

    let err = unsafe { lfs_dir_read_(lfs, dir, info) };

    lfs_util::trace!("lfs_dir_read -> {}", err);
    
    unsafe {
        if let Some(unlock_fn) = (*cfg).unlock {
            unlock_fn(cfg);
        }
    }
    
    err
}

/// 在目录中设置位置
pub fn lfs_dir_seek(lfs: *mut Lfs, dir: *mut LfsDir, off: LfsOff) -> i32 {
    // 尝试锁定文件系统
    let err = unsafe { LFS_LOCK((*lfs).cfg) };
    if err != 0 {
        return err;
    }

    // 输出跟踪信息
    lfs_util::trace!("lfs_dir_seek(%p, %p, %"PRIu32")",
            lfs as *mut c_void, dir as *mut c_void, off);

    // 调用内部实现
    let err = lfs_dir_seek_(lfs, dir, off);

    // 输出跟踪信息并解锁
    lfs_util::trace!("lfs_dir_seek -> %d", err);
    unsafe { LFS_UNLOCK((*lfs).cfg) };
    err
}

/// 返回目录的当前位置
pub fn lfs_dir_tell(lfs: *mut Lfs, dir: *mut LfsDir) -> LfsSoff {
    let err = unsafe {
        if let Some(lock) = (*(*lfs).cfg).lock {
            lock((*lfs).cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_dir_tell({:p}, {:p})", lfs, dir);

    let res = unsafe { lfs_dir_tell_(lfs, dir) };

    lfs_util::trace!("lfs_dir_tell -> {}", res);
    
    unsafe {
        if let Some(unlock) = (*(*lfs).cfg).unlock {
            unlock((*lfs).cfg);
        }
    }
    
    res
}

/// Rewinds a directory back to the beginning
pub fn lfs_dir_rewind(lfs: *mut Lfs, dir: *mut LfsDir) -> i32 {
    let err = unsafe { LFS_LOCK((*lfs).cfg) };
    if err != 0 {
        return err;
    }
    
    #[cfg(feature = "lfs_trace")]
    trace!("lfs_dir_rewind({:p}, {:p})", lfs, dir);

    let err = unsafe { lfs_dir_rewind_(lfs, dir) };

    #[cfg(feature = "lfs_trace")]
    trace!("lfs_dir_rewind -> {}", err);
    
    unsafe { LFS_UNLOCK((*lfs).cfg) };
    err
}

/// 获取文件系统状态信息
pub fn lfs_fs_stat(lfs: *mut Lfs, fsinfo: *mut LfsFsinfo) -> i32 {
    let err = unsafe { LFS_LOCK((*lfs).cfg) };
    if err != 0 {
        return err;
    }
    
    #[cfg(feature = "trace")]
    trace!("lfs_fs_stat({:p}, {:p})", lfs, fsinfo);

    let err = unsafe { lfs_fs_stat_(lfs, fsinfo) };

    #[cfg(feature = "trace")]
    trace!("lfs_fs_stat -> {}", err);
    
    unsafe { LFS_UNLOCK((*lfs).cfg) };
    err
}

/// 获取文件系统大小
pub fn lfs_fs_size(lfs: *mut Lfs) -> LfsSsize {
    // 获取锁
    let err = unsafe { LFS_LOCK((*lfs).cfg) };
    if err != 0 {
        return err;
    }
    
    // 日志
    lfs_util::trace!("lfs_fs_size(%p)", lfs as *const c_void);

    // 调用内部实现获取大小
    let res = unsafe { lfs_fs_size_(lfs) };

    // 日志和解锁
    lfs_util::trace!("lfs_fs_size -> {}", res);
    unsafe { LFS_UNLOCK((*lfs).cfg) };
    
    res
}

/// 遍历文件系统
pub unsafe fn lfs_fs_traverse(lfs: *mut Lfs, cb: Option<unsafe extern "C" fn(data: *mut c_void, block: LfsBlock) -> i32>, data: *mut c_void) -> i32 {
    let err = if let Some(lock) = (*(*lfs).cfg).lock {
        lock((*lfs).cfg)
    } else {
        0
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_fs_traverse(%p, %p, %p)",
               lfs as *mut c_void, cb.map_or(core::ptr::null_mut(), |f| f as *mut c_void), data);

    let err = lfs_fs_traverse_(lfs, cb, data, true);

    lfs_util::trace!("lfs_fs_traverse -> %d", err);
    
    if let Some(unlock) = (*(*lfs).cfg).unlock {
        unlock((*lfs).cfg);
    }
    
    err
}

/// Make file system consistent
pub fn lfs_fs_mkconsistent(lfs: *mut Lfs) -> i32 {
    let err = match unsafe { (*lfs).cfg.as_ref().unwrap().lock.unwrap()((*lfs).cfg) } {
        0 => 0,
        err => return err,
    };
    
    lfs_util::trace!("lfs_fs_mkconsistent(%p)", lfs as *const c_void);

    let err = lfs_fs_mkconsistent_(lfs);

    lfs_util::trace!("lfs_fs_mkconsistent -> {}", err);
    unsafe { (*lfs).cfg.as_ref().unwrap().unlock.unwrap()((*lfs).cfg) };
    err
}

pub fn lfs_fs_gc(lfs: &mut Lfs) -> i32 {
    let err = unsafe { 
        if let Some(lock) = (*lfs.cfg).lock {
            lock(lfs.cfg)
        } else {
            0
        }
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_fs_gc(%p)", lfs as *mut Lfs);

    let err = lfs_fs_gc_(lfs);

    lfs_util::trace!("lfs_fs_gc -> {}", err);
    
    unsafe {
        if let Some(unlock) = (*lfs.cfg).unlock {
            unlock(lfs.cfg);
        }
    }
    
    err
}

/// 增长文件系统的块数量
pub unsafe fn lfs_fs_grow(lfs: *mut Lfs, block_count: LfsSize) -> i32 {
    let err = match (*lfs).cfg.as_ref().unwrap().lock {
        Some(lock_fn) => lock_fn((*lfs).cfg),
        None => 0,
    };
    
    if err != 0 {
        return err;
    }
    
    lfs_util::trace!("lfs_fs_grow({:p}, {})", lfs, block_count);

    let err = lfs_fs_grow_(lfs, block_count);

    lfs_util::trace!("lfs_fs_grow -> {}", err);
    
    if let Some(unlock_fn) = (*lfs).cfg.as_ref().unwrap().unlock {
        unlock_fn((*lfs).cfg);
    }
    
    err
}

pub fn lfs_migrate(lfs: *mut Lfs, cfg: *const LfsConfig) -> i32 {
    let err = unsafe { LFS_LOCK(cfg) };
    if err != 0 {
        return err;
    }

    LFS_TRACE!(
        "lfs_migrate({:p}, {:p} {{.context={:p}, \
        .read={:p}, .prog={:p}, .erase={:p}, .sync={:p}, \
        .read_size={}, .prog_size={}, \
        .block_size={}, .block_count={}, \
        .block_cycles={}, .cache_size={}, \
        .lookahead_size={}, .read_buffer={:p}, \
        .prog_buffer={:p}, .lookahead_buffer={:p}, \
        .name_max={}, .file_max={}, \
        .attr_max={}}})",
        lfs,
        cfg,
        unsafe { (*cfg).context },
        unsafe { (*cfg).read.map_or(core::ptr::null(), |f| f as *const () as *const c_void) },
        unsafe { (*cfg).prog.map_or(core::ptr::null(), |f| f as *const () as *const c_void) },
        unsafe { (*cfg).erase.map_or(core::ptr::null(), |f| f as *const () as *const c_void) },
        unsafe { (*cfg).sync.map_or(core::ptr::null(), |f| f as *const () as *const c_void) },
        unsafe { (*cfg).read_size },
        unsafe { (*cfg).prog_size },
        unsafe { (*cfg).block_size },
        unsafe { (*cfg).block_count },
        unsafe { (*cfg).block_cycles },
        unsafe { (*cfg).cache_size },
        unsafe { (*cfg).lookahead_size },
        unsafe { (*cfg).read_buffer },
        unsafe { (*cfg).prog_buffer },
        unsafe { (*cfg).lookahead_buffer },
        unsafe { (*cfg).name_max },
        unsafe { (*cfg).file_max },
        unsafe { (*cfg).attr_max }
    );

    let err = unsafe { lfs_migrate_(lfs, cfg) };

    LFS_TRACE!("lfs_migrate -> {}", err);
    unsafe { LFS_UNLOCK(cfg) };
    err
}

