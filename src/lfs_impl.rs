use crate::lfs_utils::*;
use core::ffi::c_void;
use crate::lfs_strucs::*;


// 自动添加的0级依赖函数
fn lfs_alloc_ckpoint(lfs: &mut Lfs) {
    lfs.lookahead.ckpoint = lfs.block_count;
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

fn lfs_bd_read(
    lfs: &mut Lfs,
    pcache: &mut LfsCache,
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
        if block == pcache.block && current_off < pcache.off + pcache.size {
            if current_off >= pcache.off {
                // 已经在pcache中
                diff = lfs_min(diff, pcache.size - (current_off - pcache.off));
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        pcache.buffer.add((current_off - pcache.off) as usize),
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
            diff = lfs_min(diff, pcache.off - current_off);
        }

        // 检查是否在rcache中
        if block == rcache.block && current_off < rcache.off + rcache.size {
            if current_off >= rcache.off {
                // 已经在rcache中
                diff = lfs_min(diff, rcache.size - (current_off - rcache.off));
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
            diff = lfs_min(diff, rcache.off - current_off);
        }

        // 尝试直接读取，绕过缓存
        if remaining_size >= hint && current_off % cfg.read_size == 0 && remaining_size >= cfg.read_size {
            diff = lfs_aligndown(diff, cfg.read_size);
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
        rcache.off = lfs_aligndown(current_off, cfg.read_size);
        rcache.size = lfs_min(
            lfs_min(
                lfs_alignup(current_off + hint, cfg.read_size),
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

pub fn lfs_cache_drop(lfs: &mut Lfs, rcache: &mut LfsCache) {
    // do not zero, cheaper if cache is readonly or only going to be
    // written with identical data (during relocates)
    let _ = lfs; // unused parameter
    rcache.block = LFS_BLOCK_NULL;
}

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

fn lfs_ctz_fromle32(ctz: &mut LfsCtz) {
    ctz.head = lfs_fromle32(ctz.head);
    ctz.size = lfs_fromle32(ctz.size);
}

fn lfs_ctz_index(lfs: &mut Lfs, off: &mut LfsOff) -> LfsBlock {
    let size = *off;
    let b = unsafe { (*lfs.cfg).block_size } - 2 * 4;
    let mut i = size / b;
    if i == 0 {
        return 0;
    }

    i = (size - 4 * (lfs_popc(i - 1) + 2)) / b;
    *off = size - b * i - 4 * lfs_popc(i);
    return i;
}

fn lfs_ctz_tole32(ctz: &mut LfsCtz) {
    ctz.head = lfs_tole32(ctz.head);
    ctz.size = lfs_tole32(ctz.size);
}

fn lfs_deinit(lfs: &mut Lfs) -> i32 {
    // 释放已分配的内存
    unsafe {
        // 如果没有提供读缓冲区，则释放内部分配的缓冲区
        if (*lfs.cfg).read_buffer.is_null() {
            lfs_free(lfs.rcache.buffer as *mut c_void);
        }

        // 如果没有提供写入缓冲区，则释放内部分配的缓冲区
        if (*lfs.cfg).prog_buffer.is_null() {
            lfs_free(lfs.pcache.buffer as *mut c_void);
        }

        // 如果没有提供前瞻缓冲区，则释放内部分配的缓冲区
        if (*lfs.cfg).lookahead_buffer.is_null() {
            lfs_free(lfs.lookahead.buffer as *mut c_void);
        }
    }

    0
}

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

pub fn lfs_dir_tell_(lfs: *mut Lfs, dir: *mut LfsDir) -> LfsSoff {
    // 忽略lfs参数
    unsafe {
        (*dir).pos as i32
    }
}

fn lfs_fcrc_fromle32(fcrc: &mut LfsFcrc) {
    fcrc.size = lfs_fromle32(fcrc.size);
    fcrc.crc = lfs_fromle32(fcrc.crc);
}

fn lfs_fcrc_tole32(fcrc: &mut LfsFcrc) {
    fcrc.size = lfs_tole32(fcrc.size);
    fcrc.crc = lfs_tole32(fcrc.crc);
}

fn lfs_file_size_(lfs: &Lfs, file: &LfsFile) -> LfsSoff {
    // 使用lfs参数以避免未使用警告
    let _ = lfs;

    if (file.flags & LfsOpenFlags::Writing as u32) != 0 {
        return lfs_max(file.pos, file.ctz.size) as LfsSoff;
    }

    file.ctz.size as LfsSoff
}

pub unsafe fn lfs_file_tell_(lfs: *mut Lfs, file: *mut LfsFile) -> LfsSoff {
    let _ = lfs; // 未使用的参数
    (*file).pos as i32
}

fn lfs_fs_disk_version(lfs: &Lfs) -> u32 {
    if unsafe { (*lfs.cfg).disk_version != 0 } {
        return unsafe { (*lfs.cfg).disk_version };
    }
    
    LFS_DISK_VERSION
}

fn lfs_fs_prepmove(lfs: &mut Lfs, id: u16, pair: &[LfsBlock; 2]) {
    lfs.gstate.tag = (lfs.gstate.tag & !lfs_mktag(0x7ff, 0x3ff, 0)) |
        ((id != 0x3ff) as u32 * lfs_mktag(LfsType::Delete as u32, id as u32, 0));
    lfs.gstate.pair[0] = if id != 0x3ff { pair[0] } else { 0 };
    lfs.gstate.pair[1] = if id != 0x3ff { pair[1] } else { 0 };
}

fn lfs_fs_prepsuperblock(lfs: &mut Lfs, needssuperblock: bool) {
    lfs.gstate.tag = (lfs.gstate.tag & !lfs_mktag(0, 0, 0x200))
        | ((needssuperblock as u32) << 9);
}

pub unsafe fn lfs_fs_size_count(p: *mut c_void, block: LfsBlock) -> i32 {
    let _ = block; // 忽略block参数
    let size = p as *mut LfsSize;
    *size += 1;
    0
}

pub fn lfs_gstate_fromle32(a: &mut LfsGstate) {
    a.tag = lfs_fromle32(a.tag);
    a.pair[0] = lfs_fromle32(a.pair[0]);
    a.pair[1] = lfs_fromle32(a.pair[1]);
}

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

fn lfs_gstate_tole32(a: &mut LfsGstate) {
    a.tag = lfs_tole32(a.tag);
    a.pair[0] = lfs_tole32(a.pair[0]);
    a.pair[1] = lfs_tole32(a.pair[1]);
}

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

pub unsafe fn lfs_mlist_append(lfs: *mut Lfs, mlist: *mut LfsMlist) {
    (*mlist).next = (*lfs).mlist;
    (*lfs).mlist = mlist;
}

fn lfs_mlist_isopen(mut head: *mut LfsMlist, node: *const LfsMlist) -> bool {
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

pub fn lfs_pair_cmp(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> bool {
    !(paira[0] == pairb[0] || paira[1] == pairb[1] ||
      paira[0] == pairb[1] || paira[1] == pairb[0])
}

pub fn lfs_pair_fromle32(pair: &mut [LfsBlock; 2]) {
    pair[0] = lfs_fromle32(pair[0]);
    pair[1] = lfs_fromle32(pair[1]);
}

pub fn lfs_pair_isnull(pair: &[LfsBlock; 2]) -> bool {
    pair[0] == LFS_BLOCK_NULL || pair[1] == LFS_BLOCK_NULL
}

pub fn lfs_pair_issync(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> bool {
    (paira[0] == pairb[0] && paira[1] == pairb[1]) ||
    (paira[0] == pairb[1] && paira[1] == pairb[0])
}

fn lfs_pair_swap(pair: &mut [LfsBlock; 2]) {
    let t = pair[0];
    pair[0] = pair[1];
    pair[1] = t;
}

pub fn lfs_pair_tole32(pair: &mut [LfsBlock; 2]) {
    pair[0] = lfs_tole32(pair[0]);
    pair[1] = lfs_tole32(pair[1]);
}

pub fn lfs_path_namelen(path: &str) -> LfsSize {
    path.find('/').unwrap_or(path.len()) as LfsSize
}

pub fn lfs_superblock_fromle32(superblock: &mut LfsSuperblock) {
    superblock.version = lfs_fromle32(superblock.version);
    superblock.block_size = lfs_fromle32(superblock.block_size);
    superblock.block_count = lfs_fromle32(superblock.block_count);
    superblock.name_max = lfs_fromle32(superblock.name_max);
    superblock.file_max = lfs_fromle32(superblock.file_max);
    superblock.attr_max = lfs_fromle32(superblock.attr_max);
}

fn lfs_superblock_tole32(superblock: &mut LfsSuperblock) {
    superblock.version = lfs_tole32(superblock.version);
    superblock.block_size = lfs_tole32(superblock.block_size);
    superblock.block_count = lfs_tole32(superblock.block_count);
    superblock.name_max = lfs_tole32(superblock.name_max);
    superblock.file_max = lfs_tole32(superblock.file_max);
    superblock.attr_max = lfs_tole32(superblock.attr_max);
}

pub fn lfs_tag_chunk(tag: LfsTag) -> u8 {
    ((tag & 0x0ff00000) >> 20) as u8
}

pub fn lfs_tag_id(tag: LfsTag) -> u16 {
    ((tag & 0x000ffc00) >> 10) as u16
}

pub fn lfs_tag_isdelete(tag: LfsTag) -> bool {
    ((tag << 22) as i32) >> 22 == -1
}

pub fn lfs_tag_isvalid(tag: LfsTag) -> bool {
    !(tag & 0x80000000) != 0
}

pub fn lfs_tag_size(tag: LfsTag) -> LfsSize {
    tag & 0x000003ff
}

pub fn lfs_tag_type1(tag: LfsTag) -> u16 {
    ((tag & 0x70000000) >> 20) as u16
}

fn lfs_tag_type2(tag: LfsTag) -> u16 {
    ((tag & 0x78000000) >> 20) as u16
}

pub fn lfs_tag_type3(tag: LfsTag) -> u16 {
    ((tag & 0x7ff00000) >> 20) as u16
}

// 自动添加的1级依赖函数

pub fn lfs_alloc_drop(lfs: &mut Lfs) {
    lfs.lookahead.size = 0;
    lfs.lookahead.next = 0;
    lfs_alloc_ckpoint(lfs);
}

fn lfs_bd_cmp(
    lfs: &mut Lfs,
    pcache: &mut LfsCache,
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

        diff = lfs_min(size - i as LfsSize, dat.len() as LfsSize);
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

            *crc = lfs_crc(*crc, &dat[..diff as usize]);
        }
    }

    0
}

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

    let mut current = lfs_ctz_index(lfs, &(size - 1));
    let target = lfs_ctz_index(lfs, &pos);
    let mut head_value = head;

    while current > target {
        let skip = lfs_min(
            lfs_npw2(current - target + 1) - 1,
            lfs_ctz(current),
        );

        let err = lfs_bd_read(
            lfs,
            pcache,
            rcache,
            core::mem::size_of::<LfsBlock>() as LfsSize,
            head_value,
            4 * skip,
            &mut head_value as *mut _ as *mut core::ffi::c_void,
            core::mem::size_of::<LfsBlock>() as LfsSize,
        );
        
        head_value = lfs_fromle32(head_value);
        if err != 0 {
            return err;
        }

        current -= 1 << skip;
    }

    *block = head_value;
    *off = pos;
    return 0;
}

pub fn lfs_dir_close_(lfs: *mut Lfs, dir: *mut LfsDir) -> i32 {
    // 从mdir列表中移除
    unsafe {
        lfs_mlist_remove(lfs, dir as *mut LfsMlist);
    }

    0
}

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

fn lfs_fs_disk_version_major(lfs: &Lfs) -> u16 {
    0xffff & (lfs_fs_disk_version(lfs) >> 16)
}

pub fn lfs_fs_disk_version_minor(lfs: &mut Lfs) -> u16 {
    (lfs_fs_disk_version(lfs) & 0xffff) as u16
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

pub fn lfs_gstate_getorphans(a: &LfsGstate) -> u8 {
    (lfs_tag_size(a.tag) & 0x1ff) as u8
}

pub fn lfs_gstate_hasmove(a: &LfsGstate) -> bool {
    lfs_tag_type1(a.tag)
}

pub fn lfs_gstate_hasmovehere(a: &LfsGstate, pair: &[LfsBlock; 2]) -> bool {
    lfs_tag_type1(a.tag) && lfs_pair_cmp(&a.pair, pair) == 0
}

pub fn lfs_gstate_hasorphans(a: &LfsGstate) -> bool {
    lfs_tag_size(a.tag) != 0
}

pub fn lfs_gstate_needssuperblock(a: &LfsGstate) -> bool {
    lfs_tag_size(a.tag) >> 9 != 0
}

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
    debug_assert!(4 * lfs_npw2(0xffffffff / (lfs.cfg.block_size - 2 * 4))
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
        lfs.rcache.buffer = lfs_malloc(lfs.cfg.cache_size) as *mut u8;
        if lfs.rcache.buffer.is_null() {
            err = LfsError::NoMem as i32;
            goto!(cleanup);
        }
    }

    // 设置程序缓存
    if !lfs.cfg.prog_buffer.is_null() {
        lfs.pcache.buffer = lfs.cfg.prog_buffer as *mut u8;
    } else {
        lfs.pcache.buffer = lfs_malloc(lfs.cfg.cache_size) as *mut u8;
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
        lfs.lookahead.buffer = lfs_malloc(lfs.cfg.lookahead_size) as *mut u8;
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
        lfs.inline_max = lfs_min(
                lfs.cfg.cache_size,
                lfs_min(
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

pub fn lfs_path_isdir(path: &[u8]) -> bool {
    let name_len = lfs_path_namelen(path);
    path.len() > name_len && path[name_len] != 0
}

pub fn lfs_path_islast(path: &[u8]) -> bool {
    let namelen = lfs_path_namelen(path);
    let remaining = &path[namelen..];
    
    // 找到第一个非'/'字符的位置
    let slash_span = remaining.iter().take_while(|&&c| c == b'/').count();
    
    // 检查剩余部分是否全部是结束符
    remaining.get(slash_span).copied() == Some(0) || remaining.get(slash_span).is_none()
}

fn lfs_tag_dsize(tag: LfsTag) -> LfsSize {
    core::mem::size_of::<LfsTag>() as LfsSize + lfs_tag_size(tag + lfs_tag_isdelete(tag))
}

pub fn lfs_tag_splice(tag: LfsTag) -> i8 {
    lfs_tag_chunk(tag) as i8
}

fn lfs_tortoise_detectcycles(
    dir: &LfsMdir, 
    tortoise: &mut LfsTortoise,
) -> i32 {
    // 使用Brent算法检测循环
    if lfs_pair_issync(&dir.tail, &tortoise.pair) {
        lfs_WARN!("Cycle detected in tail list");
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

fn lfs_unmount_(lfs: *mut Lfs) -> i32 {
    lfs_deinit(lfs)
}


// 自动添加的2级依赖函数

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

fn lfs_dir_commit_size(p: *mut c_void, tag: LfsTag, buffer: *const c_void) -> i32 {
    let size = unsafe { &mut *(p as *mut LfsSize) };
    let _ = buffer; // 忽略未使用的参数

    // 增加size值，加上标签中指定的数据大小
    *size += lfs_tag_dsize(tag);
    0
}

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

pub fn lfs_fs_preporphans(lfs: &mut Lfs, orphans: i8) -> i32 {
    debug_assert!(lfs_tag_size(lfs.gstate.tag) > 0x000 || orphans >= 0);
    debug_assert!(lfs_tag_size(lfs.gstate.tag) < 0x1ff || orphans <= 0);
    
    lfs.gstate.tag += orphans as u32;
    lfs.gstate.tag = (lfs.gstate.tag & !LFS_MKTAG(0x800, 0, 0)) |
        ((lfs_gstate_hasorphans(&lfs.gstate) as u32) << 31);

    0
}

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


// 自动添加的2级依赖函数

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

fn lfs_fs_parent(lfs: *mut Lfs, pair: &[LfsBlock; 2], parent: *mut LfsMdir) -> LfsStag {
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
