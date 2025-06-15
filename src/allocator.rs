use super::defines::*;
use super::utils::*;
use super::lfs::*;

pub fn lfs_alloc_lookahead(lfs_instance: &mut Lfs, block: LfsBlock) -> Result<(), LfsError> {
    let cfg = unsafe { &*lfs_instance.cfg };// Get config ref

    let off = (block.wrapping_sub(lfs_instance.free.start)) % cfg.block_count;
    if off < cfg.lookahead {
        // Safety: lfs_instance.free.lookahead must point to a valid buffer
        // of size (cfg.lookahead / 32) u32 elements.
        let lookahead_slice = unsafe {
            core::slice::from_raw_parts_mut(
                lfs_instance.free.lookahead,
                (cfg.lookahead / 32) as usize,
            )
        };
        let index = (off / 32) as usize;
        let bit = (off % 32) as u32;
        if index < lookahead_slice.len() {
            lookahead_slice[index] |= 1u32 << bit;
        } else {
            // This case should ideally not happen if cfg.lookahead and buffer size are consistent
            // and 'off' is correctly calculated.
            return Err(LfsError::Inval); // Or some other error indicating inconsistency
        }
    }
    Ok(())
}

pub fn lfs_alloc(lfs: &mut Lfs) -> Result<LfsBlock, LfsError> {
    if !lfs.deorphaned {
        lfs_deorphan(lfs)?;
        // lfs.deorphaned would be set to true by lfs_deorphan
    }

    loop {
        loop {
            // Check if we have looked at all blocks since last ack
            if lfs.free.start+ lfs.free.off == lfs.free.end {
                return Err(LfsError::NoSpc);
            }

            let cfg = unsafe { &*lfs.cfg };
            if lfs.free.off >= cfg.lookahead {
                break; // Need to repopulate lookahead
            }

            let current_lookahead_offset = lfs.free.off;
            lfs.free.off = lfs.free.off.wrapping_add(1);

            // Safety: lfs.free.lookahead must point to a valid buffer
            let lookahead_slice = unsafe {
                core::slice::from_raw_parts_mut(
                    lfs.free.lookahead,
                    (cfg.lookahead / 32) as usize,
                )
            };
            let index = (current_lookahead_offset / 32) as usize;
            let bit = (current_lookahead_offset % 32) as u32;

            if index < lookahead_slice.len() {
                if (lookahead_slice[index] & (1u32 << bit)) == 0 {
                    // Found a free block
                    let allocated_block = (lfs.free.start.wrapping_add(current_lookahead_offset)) % cfg.block_count;
                    return Ok(allocated_block);
                }
            } else {
                // Should not happen with correct setup
                return Err(LfsError::Inval);
            }
        }

        // Repopulate lookahead buffer
        let cfg = unsafe { &*lfs.cfg };
        lfs.free.start = lfs.free.start.wrapping_add(cfg.lookahead);
        lfs.free.off = 0;

        // Clear the lookahead buffer
        // Safety: lfs.free.lookahead must point to a valid buffer
        let lookahead_slice_mut = unsafe {
            core::slice::from_raw_parts_mut(
                lfs.free.lookahead,
                (cfg.lookahead / 32) as usize, // Number of u32 elements
            )
        };
        for elem in lookahead_slice_mut.iter_mut() {
            *elem = 0;
        }
        // The C code uses memset with size cfg.lookahead / 8 (bytes).
        // Since lookahead is *mut u32, this means (cfg.lookahead / 32) u32 elements.

        // Find mask of free blocks from tree
        lfs_traverse(lfs, lfs_alloc_lookahead)?;
    }
}

pub fn lfs_alloc_ack(lfs: &mut Lfs) {
    let cfg = unsafe { &*lfs.cfg };
    lfs.free.end = lfs.free.start
        .wrapping_add(lfs.free.off)
        .wrapping_add(cfg.block_count);
}