use super::defines::*;
use super::utils::*;

pub fn lfs_cache_read(
    cfg: &LfsConfig,
    rcache: &mut LfsCache,
    pcache: Option<&LfsCache>,
    block: LfsBlock,
    off: LfsOff,
    buffer_out: &mut [u8],
) -> Result<(), LfsError> {
    let mut current_off = off;
    let mut total_size_to_read = buffer_out.len() as LfsSize;
    let mut buffer_slice = buffer_out;

    while total_size_to_read > 0 {
        // Try reading from program cache (pcache)
        if let Some(pc) = pcache {
            if pc.block == block && current_off >= pc.off && current_off < pc.off + cfg.prog_size {
                let pcache_off = (current_off - pc.off) as usize;
                let pcache_readable_len = (cfg.prog_size - (current_off - pc.off)) as usize;
                let diff = lfs_min(total_size_to_read, pcache_readable_len as LfsSize) as usize;

                let pcache_buffer = unsafe {
                    // Safety: pc.buffer must be valid for cfg.prog_size
                    core::slice::from_raw_parts(pc.buffer, cfg.prog_size as usize)
                };
                buffer_slice[..diff].copy_from_slice(&pcache_buffer[pcache_off..pcache_off + diff]);

                current_off += diff as LfsOff;
                total_size_to_read -= diff as LfsSize;
                buffer_slice = &mut buffer_slice[diff..];
                continue;
            }
        }

        // Try reading from read cache (rcache)
        if rcache.block == block && current_off >= rcache.off && current_off < rcache.off + cfg.read_size {
            let rcache_off = (current_off - rcache.off) as usize;
            let rcache_readable_len = (cfg.read_size - (current_off - rcache.off)) as usize;
            let diff = lfs_min(total_size_to_read, rcache_readable_len as LfsSize) as usize;

            let rcache_buffer_data = unsafe {
                // Safety: rcache.buffer must be valid for cfg.read_size
                core::slice::from_raw_parts(rcache.buffer, cfg.read_size as usize)
            };
            buffer_slice[..diff].copy_from_slice(&rcache_buffer_data[rcache_off..rcache_off + diff]);

            current_off += diff as LfsOff;
            total_size_to_read -= diff as LfsSize;
            buffer_slice = &mut buffer_slice[diff..];
            continue;
        }

        // Read directly from device if aligned and size is sufficient (bypass cache)
        if current_off % cfg.read_size == 0 && total_size_to_read >= cfg.read_size {
            let diff = total_size_to_read - (total_size_to_read % cfg.read_size);
            if let Some(read_fn) = cfg.read {
                let err = read_fn(cfg, block, current_off, buffer_slice.as_mut_ptr(), diff);
                if err != 0 {
                    return Err(unsafe { core::mem::transmute(err) });
                }
            } else {
                return Err(LfsError::Io); // Or some other appropriate error
            }

            current_off += diff;
            total_size_to_read -= diff;
            buffer_slice = &mut buffer_slice[diff as usize..];
            continue;
        }

        // Load to read cache (rcache)
        rcache.block = block;
        rcache.off = current_off - (current_off % cfg.read_size);
        
        let rcache_buffer_mut = unsafe {
            // Safety: rcache.buffer must be valid for cfg.read_size
            core::slice::from_raw_parts_mut(rcache.buffer, cfg.read_size as usize)
        };

        if let Some(read_fn) = cfg.read {
            let err = read_fn(cfg, rcache.block, rcache.off, rcache_buffer_mut.as_mut_ptr(), cfg.read_size);
            if err != 0 {
                return Err(unsafe { core::mem::transmute(err) });
            }
        } else {
            return Err(LfsError::Io); // Or some other appropriate error
        }
        // The loop will then attempt to read from rcache in the next iteration
    }
    Ok(())
}

pub fn lfs_cache_cmp(
    cfg: &LfsConfig,
    rcache: &mut LfsCache,
    pcache: Option<&LfsCache>,
    block: LfsBlock,
    off: LfsOff,
    buffer_to_compare: &[u8],
) -> Result<bool, LfsError> {
    for i in 0..buffer_to_compare.len() {
        let mut c_byte = [0u8; 1];
        lfs_cache_read(cfg, rcache, pcache, block, off + i as LfsOff, &mut c_byte)?;
        if c_byte[0] != buffer_to_compare[i] {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn lfs_cache_crc(
    cfg: &LfsConfig,
    rcache: &mut LfsCache,
    pcache: Option<&LfsCache>,
    block: LfsBlock,
    off: LfsOff,
    size: LfsSize,
    crc_val: &mut u32,
) -> Result<(), LfsError> {
    for i in 0..size {
        let mut c_byte = [0u8; 1];
        lfs_cache_read(cfg, rcache, pcache, block, off + i, &mut c_byte)?;
        lfs_crc(crc_val, &c_byte);
    }
    Ok(())
}

pub fn lfs_cache_flush(
    cfg: &LfsConfig, // &mut Lfs if cfg->context might be changed by prog
    pcache: &mut LfsCache,
    rcache: Option<&mut LfsCache>, // Made Option<&mut LfsCache> if cache_cmp updates it
) -> Result<(), LfsError> {

    if pcache.block != 0xffffffff { // 0xffffffff typically means invalid/empty
        let pcache_buffer = unsafe {
            core::slice::from_raw_parts(pcache.buffer, cfg.prog_size as usize)
        };
        if let Some(prog_fn) = cfg.prog {
             let err = prog_fn(cfg, pcache.block, pcache.off, pcache_buffer.as_ptr(), cfg.prog_size);
            if err != 0 {
                // LFS_ERR_CORRUPT is a special case that prog can return
                return Err(unsafe{ core::mem::transmute(err) });
            }
        } else {
            return Err(LfsError::Io);
        }

        if let Some(rc) = rcache {
            // Verify write if rcache is provided
            match lfs_cache_cmp(cfg, rc, None, pcache.block, pcache.off, pcache_buffer) {
                Ok(true) => { /* Match */ }
                Ok(false) => return Err(LfsError::Corrupt),
                Err(e) => return Err(e),
            }
        }
        pcache.block = 0xffffffff; // Mark pcache as flushed
    }
    Ok(())
}

pub fn lfs_cache_prog(
    cfg: &LfsConfig,
    pcache: &mut LfsCache,
    mut rcache: Option<&mut LfsCache>,
    block: LfsBlock,
    off: LfsOff,
    buffer_in: &[u8],
) -> Result<(), LfsError> {
    let mut current_off = off;
    let mut total_size_to_prog = buffer_in.len() as LfsSize;
    let mut buffer_slice = buffer_in;

    while total_size_to_prog > 0 {
        // Program to pcache if it's the right block and offset
        if pcache.block == block && current_off >= pcache.off && current_off < pcache.off + cfg.prog_size {
            let pcache_internal_off = (current_off - pcache.off) as usize;
            let pcache_programmable_len = (cfg.prog_size - (current_off - pcache.off)) as usize;
            let diff = lfs_min(total_size_to_prog, pcache_programmable_len as LfsSize) as usize;

            let pcache_buffer_mut = unsafe {
                core::slice::from_raw_parts_mut(pcache.buffer, cfg.prog_size as usize)
            };
            pcache_buffer_mut[pcache_internal_off..pcache_internal_off + diff]
                .copy_from_slice(&buffer_slice[..diff]);

            current_off += diff as LfsOff;
            total_size_to_prog -= diff as LfsSize;
            buffer_slice = &buffer_slice[diff..];

            // Eagerly flush if pcache fills up
            if current_off % cfg.prog_size == 0 { // Or if pcache_internal_off + diff == cfg.prog_size

                lfs_cache_flush(cfg, pcache, rcache.as_mut().map(|r| &mut **r))?;

            }
            continue;
        }

        // Bypass pcache? Program directly to device if aligned and size is sufficient
        if current_off % cfg.prog_size == 0 && total_size_to_prog >= cfg.prog_size {
            let diff = total_size_to_prog - (total_size_to_prog % cfg.prog_size);
            if let Some(prog_fn) = cfg.prog {
                let err = prog_fn(cfg, block, current_off, buffer_slice.as_ptr(), diff);
                 if err != 0 {
                    return Err(unsafe{core::mem::transmute(err)});
                }
            } else {
                return Err(LfsError::Io);
            }

            if let Some(rc) = rcache.as_mut().map(|r| &mut **r) { // Also very unsafe for similar reasons
                 match lfs_cache_cmp(cfg, rc, None, block, current_off, &buffer_slice[..diff as usize]) {
                    Ok(true) => { /* Match */ }
                    Ok(false) => return Err(LfsError::Corrupt),
                    Err(e) => return Err(e),
                }
            }

            current_off += diff;
            total_size_to_prog -= diff;
            buffer_slice = &buffer_slice[diff as usize..];
            continue;
        }

        // Prepare pcache for new data (implies pcache was flushed or is new)
        // This path is hit if data doesn't fit existing pcache window and can't be directly programmed.
        // The C code suggests pcache must have been flushed.
        // If we reach here, it means we need to load new data into pcache, but pcache might contain unsaved data.
        // The original C code for this else branch:
        //      pcache->block = block;
        //      pcache->off = off - (off % lfs->cfg->prog_size);
        // This effectively overwrites pcache metadata. If pcache had unflushed data for a *different* block, it's lost.
        // This path should only be taken if pcache is clean or has been flushed.
        // The C logic: "pcache must have been flushed, either by programming and entire block or manually flushing the pcache"
        // If it's not flushed and we are here, it's an issue. We assume it's flushed.
        pcache.block = block;
        pcache.off = current_off - (current_off % cfg.prog_size);
        // The loop will then attempt to write to pcache in the next iteration.
        // Ensure the pcache is empty before this by flushing, if it contained other data.
        // The C code doesn't explicitly flush here, it assumes previous logic paths handled it or it's an initial write.
    }
    Ok(())
}


// --- General lfs block device operations ---
// These are thin wrappers around the cache operations, using Lfs's internal caches.

pub fn lfs_bd_read(
    lfs: &mut Lfs, // rcache is part of Lfs and is mutable
    block: LfsBlock,
    off: LfsOff,
    buffer_out: &mut [u8],
) -> Result<(), LfsError> {
    // In C: return lfs_cache_read(lfs, &lfs->rcache, NULL, block, off, buffer, size);
    // The `NULL` for pcache means `None`.
    let cfg_ref = unsafe { &*lfs.cfg };
    lfs_cache_read(cfg_ref, &mut lfs.rcache, None, block, off, buffer_out)
}

pub fn lfs_bd_prog(
    lfs: &mut Lfs, // pcache is part of Lfs and is mutable
    block: LfsBlock,
    off: LfsOff,
    buffer_in: &[u8],
) -> Result<(), LfsError> {
    // In C: return lfs_cache_prog(lfs, &lfs->pcache, NULL, block, off, buffer, size);
    // The `NULL` for rcache means `None` for verification.
    let cfg_ref = unsafe { &*lfs.cfg };
    lfs_cache_prog(cfg_ref, &mut lfs.pcache, None, block, off, buffer_in)
}

pub fn lfs_bd_cmp(
    lfs: &mut Lfs, // rcache is part of Lfs and is mutable for reads
    block: LfsBlock,
    off: LfsOff,
    buffer_to_compare: &[u8],
) -> Result<bool, LfsError> {
    // In C: return lfs_cache_cmp(lfs, &lfs->rcache, NULL, block, off, buffer, size);
    let cfg_ref = unsafe { &*lfs.cfg };
    lfs_cache_cmp(cfg_ref, &mut lfs.rcache, None, block, off, buffer_to_compare)
}

pub fn lfs_bd_crc(
    lfs: &mut Lfs, // rcache is part of Lfs and is mutable for reads
    block: LfsBlock,
    off: LfsOff,
    size: LfsSize,
    crc_val: &mut u32,
) -> Result<(), LfsError> {
    // In C: return lfs_cache_crc(lfs, &lfs->rcache, NULL, block, off, size, crc);
    let cfg_ref = unsafe { &*lfs.cfg };
    lfs_cache_crc(cfg_ref, &mut lfs.rcache, None, block, off, size, crc_val)
}

pub fn lfs_bd_erase(lfs: &Lfs, block: LfsBlock) -> Result<(), LfsError> {
    // In C: return lfs->cfg->erase(lfs->cfg, block);
    let cfg = unsafe { &*lfs.cfg };
    if let Some(erase_fn) = cfg.erase {
        let err = erase_fn(lfs.cfg, block);
        if err != 0 {
            return Err(unsafe{core::mem::transmute(err)});
        }
        Ok(())
    } else {
        Err(LfsError::Io) // Or other suitable error
    }
}

pub fn lfs_bd_sync(lfs: &mut Lfs) -> Result<(), LfsError> {
    // In C:
    // lfs->rcache.block = 0xffffffff;
    // err = lfs_cache_flush(lfs, &lfs->pcache, NULL);
    // ...
    // return lfs->cfg->sync(lfs->cfg);

    let cfg_ref = unsafe { &*lfs.cfg };
    lfs.rcache.block = 0xffffffff; // Invalidate read cache

    // The `NULL` for rcache in lfs_cache_flush means no verification cache.
    lfs_cache_flush(cfg_ref, &mut lfs.pcache, None)?;

    let cfg = unsafe { &*lfs.cfg };
    if let Some(sync_fn) = cfg.sync {
        let err = sync_fn(lfs.cfg);
        if err != 0 {
             return Err(unsafe{core::mem::transmute(err)});
        }
        Ok(())
    } else {
        Err(LfsError::Io) // Or other suitable error
    }
}

// Assuming the types Lfs, LfsConfig, LfsBlock, LfsError, LfsFree, etc.
// are defined as in the code you provided in the previous message.

