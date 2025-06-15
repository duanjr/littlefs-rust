use super::defines::*;
use super::utils::*;
use super::block_device::*;
use super::allocator::*;
use super::metadata_pair::*;
use super::dir::*;

pub fn lfs_index(cfg: &LfsConfig, offset_in_out: &mut LfsOff) -> LfsOff {
    let mut i: LfsOff = 0;
    let words_per_block: LfsSize = cfg.block_size / 4;

    let mut current_offset = *offset_in_out;
    while current_offset >= cfg.block_size {
        i += 1;
        current_offset -= cfg.block_size;
        // The number of pointers in an index block is related to lfs_ctz(i)+1
        // but capped by words_per_block-1.
        // Each pointer "covers" block_size bytes, but the pointer itself takes 4 bytes.
        // So, effectively, block_size is reduced by these pointer indirections.
        current_offset += 4 * lfs_min(lfs_ctz(i).wrapping_add(1), words_per_block.saturating_sub(1));
    }
    *offset_in_out = current_offset;
    i
}

pub fn lfs_index_find(
    cfg: &LfsConfig,
    rcache: &mut LfsCache,
    pcache: Option<&LfsCache>,
    mut head_block: LfsBlock, // Start block of the file's index list
    file_size: LfsSize,       // Total size of the file
    target_pos_in_file: LfsSize, // Logical position in the file to find
) -> Result<(LfsBlock, LfsOff), LfsError> {
    if file_size == 0 {
        return Ok((0xffffffff, 0)); // Indicate invalid block if file is empty
    }

    let mut effective_file_size_for_index = file_size.saturating_sub(1);
    let mut current_level = lfs_index(cfg, &mut effective_file_size_for_index);

    let mut pos_for_index_level = target_pos_in_file;
    let target_level = lfs_index(cfg, &mut pos_for_index_level);
    // pos_for_index_level is now the offset within the target_level block.

    let words_per_block = cfg.block_size / 4;

    while current_level > target_level {
        // Calculate how many levels to skip down.
        // npw2 gives roughly log2 of (levels_to_descend + 1).
        let levels_to_descend = current_level - target_level;
        let skip_power = lfs_npw2(levels_to_descend.wrapping_add(1)).saturating_sub(1);

        // Determine max pointers available at current_level's block structure.
        // The number of pointers is min(ctz(current_level)+1, words_per_block-1).
        // The skip index must be less than this.
        let max_pointers_at_level = lfs_min(lfs_ctz(current_level).wrapping_add(1), words_per_block.saturating_sub(1));
        let pointer_index_to_read = lfs_min(skip_power, max_pointers_at_level.saturating_sub(1));

        let mut next_head_block_bytes = [0u8; 4];
        lfs_cache_read(
            cfg,
            rcache,
            pcache,
            head_block,
            4 * pointer_index_to_read, // Offset of the pointer in the index block
            &mut next_head_block_bytes,
        )?;
        head_block = LfsBlock::from_le_bytes(next_head_block_bytes);

        current_level -= 1 << pointer_index_to_read; // Descend levels
    }

    Ok((head_block, pos_for_index_level))
}

pub fn lfs_index_extend(
    lfs: &mut Lfs, // Needed for lfs_alloc
    rcache: &mut LfsCache,
    pcache: &mut LfsCache,
    current_head_block: LfsBlock,
    current_file_size_at_head: LfsSize,
) -> Result<(LfsBlock, LfsOff), LfsError> {
    let cfg = unsafe { &*lfs.cfg };

    'relocate_attempt: loop {
        let new_block = lfs_alloc(lfs)?;
        match lfs_bd_erase(lfs, new_block) {
            Ok(_) => {}
            Err(LfsError::Corrupt) => { // Erase failed, block is bad
                // In C: goto relocate (which clears pcache and tries new_block)
                // Here, we loop and lfs_alloc will hopefully give a different block.
                // If lfs_alloc keeps giving the same bad block, this could loop.
                // LFS's allocator should ideally not return a block marked bad by erase.
                // The pcache clear from C's relocate is important if pcache held state for `new_block`.
                pcache.block = 0xffffffff;
                continue 'relocate_attempt;
            }
            Err(e) => return Err(e),
        }

        if current_file_size_at_head == 0 {
            // File was empty, this new_block is the first data block.
            return Ok((new_block, 0)); // Offset to write data is 0.
        }

        // Calculate index level of the (now previous) last part of the file.
        let mut size_for_index_calc = current_file_size_at_head.saturating_sub(1);
        let mut index_level = lfs_index(cfg, &mut size_for_index_calc);
        // size_for_index_calc is now offset within the block at current_head_block for that level.
        
        // If the current_head_block was not completely full, copy its contents to the new_block.
        // The `size_for_index_calc` here (after lfs_index) is the used size in the *last data block if it was head_block*.
        // The C parameter `size` maps to `current_file_size_at_head`.
        // `offset_in_last_block = current_file_size_at_head % cfg.block_size` (if it's a simple flat file structure)
        // or `size_for_index_calc` if `index_level` is 0.
        // The C logic `if (size != lfs->cfg->block_size)` refers to `size` argument of `lfs_index_extend`.
        // This `size` is the effective size of data within the `current_head_block` if it's a data block.
        // More accurately, it's `(current_file_size_at_head - 1) % cfg.block_size + 1` if `index_level == 0`.
        // Or, the `size_for_index_calc` from above if `index_level == 0` and the block was not a full block.

        let data_offset_in_head_block = size_for_index_calc; // This is offset if index_level==0
        let data_size_in_head_block = if index_level == 0 {
            // If it's a direct data block, data_offset_in_head_block is the actual data size.
            data_offset_in_head_block
        } else {
            // If current_head_block was an index block, it should be full of pointers (cfg.block_size).
            // This branch implies current_head_block was a data block not fully utilized.
            cfg.block_size // Or some other logic if head was index
        };


        if index_level == 0 && data_offset_in_head_block != cfg.block_size {
            // Last block (current_head_block) was a data block and not full. Copy its contents.
            for i in 0..data_offset_in_head_block {
                let mut byte_data = [0u8; 1];
                // Read from old head (pcache = None, as we are reading committed data)
                lfs_cache_read(cfg, rcache, None, current_head_block, i, &mut byte_data)?;
                // Program to new block
                match lfs_cache_prog(cfg, pcache, Some(rcache), new_block, i, &byte_data) {
                    Ok(_) => {}
                    Err(LfsError::Corrupt) => { pcache.block = 0xffffffff; continue 'relocate_attempt; }
                    Err(e) => return Err(e),
                }
            }
            return Ok((new_block, data_offset_in_head_block)); // Next write is after copied data
        }

        // Append new block as an index block (or as a new data block if extending a full one).
        index_level += 1; // Level of the new index structure being created / new block in sequence.
        let words_per_block = cfg.block_size / 4;
        // `skips` is the number of pointers this new index block will contain.
        let skips = lfs_min(lfs_ctz(index_level).wrapping_add(1), words_per_block.saturating_sub(1));

        let mut current_ptr_target = current_head_block; // Pointer target starts as the old head.

        for i in 0..skips {
            let ptr_bytes = current_ptr_target.to_le_bytes();
            match lfs_cache_prog(cfg, pcache, Some(rcache), new_block, 4 * i, &ptr_bytes) {
                 Ok(_) => {}
                 Err(LfsError::Corrupt) => { pcache.block = 0xffffffff; continue 'relocate_attempt; }
                 Err(e) => return Err(e),
            }

            if i != skips - 1 {
                // Read the next pointer from the chain we are replicating.
                let mut next_ptr_bytes = [0u8; 4];
                lfs_cache_read(cfg, rcache, None, current_ptr_target, 4 * i, &mut next_ptr_bytes)?;
                current_ptr_target = LfsBlock::from_le_bytes(next_ptr_bytes);
            }
        }
        return Ok((new_block, 4 * skips)); // Next write is after the pointers.
    } // End 'relocate_attempt loop
}

pub fn lfs_index_traverse<UserDataT>(
    cfg: &LfsConfig,
    rcache: &mut LfsCache,
    pcache: Option<&LfsCache>,
    mut head_block: LfsBlock,
    file_size: LfsSize,
    callback: &mut dyn FnMut(&mut UserDataT, LfsBlock) -> Result<(), LfsError>,
    callback_data: &mut UserDataT,
) -> Result<(), LfsError> {
    if file_size == 0 || head_block == 0xffffffff {
        return Ok(());
    }

    let mut size_for_index_calc = file_size.saturating_sub(1);
    let mut current_index_level = lfs_index(cfg, &mut size_for_index_calc);

    loop {
        callback(callback_data, head_block)?;

        if current_index_level == 0 {
            return Ok(()); // Reached the lowest level (data block or first index block)
        }

        let mut next_head_block_bytes = [0u8; 4];
        // In a CTZ skip list, the first pointer (offset 0) always points to the next block
        // at the immediately lower level of the same conceptual "spine".
        lfs_cache_read(cfg, rcache, pcache, head_block, 0, &mut next_head_block_bytes)?;
        head_block = LfsBlock::from_le_bytes(next_head_block_bytes);

        current_index_level -= 1;
    }
}

pub fn lfs_file_open(
    lfs: &mut Lfs,
    file_out: &mut LfsFile, // Caller-allocated LfsFile struct to be initialized
    path_str: &str,
    flags: u32,
) -> Result<(), LfsError> {
    let cfg = unsafe { &*lfs.cfg };

    // Separate path into parent_path and file_name
    let (parent_path_str, file_name_str) = {
        if let Some(idx) = path_str.rfind('/') {
            // Ensure parent_path_str includes the trailing slash if not root, or is "/" for root
            let (p_path, name) = path_str.split_at(idx);
            if p_path.is_empty() { // e.g. "/file"
                ("/", &name[1..])
            } else { // e.g. "dir/file" or "/dir/file"
                (p_path, &name[1..])
            }
        } else {
            ("", path_str) // Relative to current (root for now, as cwd is fetched from root)
        }
    };

    if file_name_str.is_empty() || file_name_str == "." || file_name_str == ".." {
        return Err(LfsError::Inval);
    }
    if file_name_str.len() > LFS_NAME_MAX {
        return Err(LfsError::Inval); // Name too long
    }

    // Fetch parent directory
    let mut cwd = LfsDir { // For the parent directory
        pair: [0,0], off: 0, head: [0,0], pos: 0,
        d: LfsDiskDir { rev: 0, size: 0, tail: [0,0] },
    };
    // Open parent directory. If path is "file.txt", parent_path_str is "", open "/"
    lfs_dir_open(lfs, &mut cwd, if parent_path_str.is_empty() { "/" } else { parent_path_str })?;

    let mut entry = LfsEntry {
        off: 0,
        d: LfsDiskEntry { type_0: 0, elen: 0, alen: 0, nlen: 0, u: LfsDiskEntryUnion { file: LfsDiskEntryFile {head:0, size:0}} }
    };
    let mut find_path_ref: &str = file_name_str;

    match lfs_dir_find(lfs, &mut cwd, &mut entry, &mut find_path_ref) {
        Ok(_) => { // Entry found
            if entry.d.type_0 == LfsType::Dir as u8 {
                return Err(LfsError::IsDir);
            }
            if (flags & LfsOpenFlags::EXCL) != 0 {
                return Err(LfsError::Exists);
            }
        }
        Err(LfsError::NoEnt) => {
            if (flags & LfsOpenFlags::CREAT) == 0 {
                return Err(LfsError::NoEnt);
            }
            // Create new file entry
            entry.d.type_0 = LfsType::Reg as u8;
            entry.d.elen = (core::mem::size_of::<LfsBlock>() + core::mem::size_of::<LfsSize>()) as u8; // sizeof(u.file)
            entry.d.alen = 0;
            entry.d.nlen = file_name_str.len() as u8;
            entry.d.u.file = LfsDiskEntryFile { head: 0xffffffff, size: 0 }; // New empty file

            // `lfs_dir_append` will set entry.off
            lfs_dir_append(lfs, &mut cwd, &mut entry, file_name_str.as_bytes())?;
        }
        Err(e) => return Err(e),
    }

    // Setup file_out struct
    file_out.pair[0] = cwd.pair[0];
    file_out.pair[1] = cwd.pair[1];
    file_out.poff = entry.off;
    file_out.head = unsafe { entry.d.u.file.head };
    file_out.size = unsafe { entry.d.u.file.size };
    file_out.flags = flags;
    file_out.pos = 0;
    // file_out.owns_cache_buffer should be initialized here

    if (flags & LfsOpenFlags::TRUNC) != 0 {
        file_out.head = 0xffffffff;
        file_out.size = 0;
    }

    // Allocate buffer if needed
    file_out.cache.block = 0xffffffff;
    file_out.cache.off = 0; // Initialize cache offset

    if !cfg.file_buffer.is_null() {
        file_out.cache.buffer = cfg.file_buffer;
        // file_out.owns_cache_buffer = false; // Assuming LfsFile struct has this field
    } else {
        let buffer_size = if (file_out.flags & LfsOpenFlags::RDWR) == LfsOpenFlags::RDONLY { // Check if RDONLY (==1)
            cfg.read_size
        } else {
            cfg.prog_size
        };
        if buffer_size == 0 { return Err(LfsError::Inval); } // Avoid zero-size allocation

        let mut buffer_vec = vec![0u8; buffer_size as usize];
        file_out.cache.buffer = Box::into_raw(buffer_vec.into_boxed_slice()) as *mut u8;
        // file_out.owns_cache_buffer = true; // Assuming LfsFile struct has this field
        if file_out.cache.buffer.is_null() { // Should not happen with vec then Box if allocation succeeds
            return Err(LfsError::NoMem);
        }
    }
    
    // Add to list of files (unsafe raw pointer manipulation)
    unsafe {
        file_out.next = lfs.files;
        lfs.files = file_out as *mut LfsFile;
    }

    Ok(())
}

pub fn lfs_file_close(lfs: &mut Lfs, file_to_close: &mut LfsFile) -> Result<(), LfsError> {
    let sync_result = lfs_file_sync(lfs, file_to_close);

    // Remove from list of files (unsafe raw pointer manipulation)
    unsafe {
        let mut current_ptr = &mut lfs.files as *mut (*mut LfsFile);
        while !(*current_ptr).is_null() {
            if *current_ptr == file_to_close as *mut LfsFile {
                *current_ptr = file_to_close.next; // Bypass file_to_close
                break;
            }
            // Check if (*current_ptr) is valid before dereferencing for next. This is tricky.
            // This loop structure needs (*(*current_ptr)).next. If *current_ptr becomes null, it's ok.
            // If *current_ptr is file_to_close, then (*current_ptr).next is file_to_close.next.
            // current_ptr = &mut (**current_ptr).next; // This is how you'd advance the pointer to the next field.
            if !(*current_ptr).is_null() { // Check again after potential modification
                 current_ptr = &mut (*(*current_ptr)).next;
            } else {
                break; // Should have been found or list ended
            }
        }
    }
    
    // Clean up memory for the cache buffer if LFS allocated it
    let cfg = unsafe { &*lfs.cfg };
    if cfg.file_buffer.is_null() { // && file_to_close.owns_cache_buffer (if using such a flag)
        if !file_to_close.cache.buffer.is_null() {
            // Determine original size to correctly reconstruct Box<[u8]>
            let buffer_size = if (file_to_close.flags & LfsOpenFlags::RDWR) == LfsOpenFlags::RDONLY {
                cfg.read_size
            } else {
                cfg.prog_size
            };
            unsafe {
                let _ = Box::from_raw(core::slice::from_raw_parts_mut(file_to_close.cache.buffer, buffer_size as usize));
            }
            file_to_close.cache.buffer = core::ptr::null_mut();
        }
    }

    sync_result // Return the result of lfs_file_sync
}

pub fn lfs_file_relocate(lfs: &mut Lfs, file: &mut LfsFile) -> Result<(), LfsError> {
    let cfg = unsafe { &*lfs.cfg };
    'relocate_attempt: loop {
        let new_block = lfs_alloc(lfs)?;
        match lfs_bd_erase(lfs, new_block) {
            Ok(_) => {}
            Err(LfsError::Corrupt) => { lfs.pcache.block = 0xffffffff; continue 'relocate_attempt; }
            Err(e) => return Err(e),
        }

        // Copy data from old block (via file.block and file.cache if dirty) to new_block (via lfs.pcache)
        for i in 0..file.off { // file.off is current data size in file.block/file.cache
            let mut byte_data = [0u8; 1];
            // Read from potentially dirty file.cache (pcache role) or file.block (rcache role for lfs.rcache)
            lfs_cache_read(cfg, &mut lfs.rcache, Some(&file.cache), file.block, i, &mut byte_data)?;
            
            match lfs_cache_prog(cfg, &mut lfs.pcache, Some(&mut lfs.rcache), new_block, i, &byte_data) {
                Ok(_) => {}
                Err(LfsError::Corrupt) => { lfs.pcache.block = 0xffffffff; continue 'relocate_attempt; }
                Err(e) => return Err(e),
            }
        }

        // Copy over new state of file (lfs.pcache now has content for new_block) to file.cache
        if !lfs.pcache.buffer.is_null() && !file.cache.buffer.is_null() {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    lfs.pcache.buffer,
                    file.cache.buffer,
                    cfg.prog_size as usize,
                );
            }
        }
        file.cache.block = lfs.pcache.block; // This should be new_block
        file.cache.off = lfs.pcache.off;    // This should be file.off after copy
        lfs.pcache.block = 0xffffffff;      // Invalidate global pcache

        file.block = new_block; // File now resides on new_block
        return Ok(());
    }
}

pub fn lfs_file_flush(lfs: &mut Lfs, file: &mut LfsFile) -> Result<(), LfsError> {
    if (file.flags & LfsOpenFlags::F_READING) != 0 {
        file.cache.block = 0xffffffff; // Invalidate file's read cache content
        file.flags &= !LfsOpenFlags::F_READING;
    }

    if (file.flags & LfsOpenFlags::F_WRITING) != 0 {
        let original_pos = file.pos;
        let cfg = unsafe { &*lfs.cfg };

        // If writing in the middle of the file, copy trailing data
        if file.pos < file.size {
            // Create a temporary LfsFile struct to read existing data
            // This is complex because LfsFile contains a cache buffer.
            // The C code creates a temporary 'orig' LfsFile on stack,
            // pointing its cache to lfs.rcache.
            let mut orig_file_for_read = LfsFile {
                next: core::ptr::null_mut(), // Not part of any list
                pair: file.pair, // Same parent dir
                poff: file.poff,
                head: file.head, // Original head and size
                size: file.size,
                flags: LfsOpenFlags::RDONLY, // Read-only for this temp purpose
                pos: file.pos, // Start reading from current write position
                block: 0,      // Will be found by lfs_index_find
                off: 0,
                cache: LfsCache { // Use global rcache temporarily for reading
                    block: 0xffffffff,
                    off: 0,
                    buffer: lfs.rcache.buffer, // Point to global read cache buffer
                },
                // owns_cache_buffer: false, // Assuming this field exists
            };
            
            // Invalidate global rcache before using it for orig_file_for_read
            // to avoid conflicts if lfs.rcache was pointed to by file.cache for some reason.
            // This is not explicitly done in C here, but good practice.
            // However, file.cache uses its own buffer or cfg.file_buffer. lfs.rcache is separate.
            // The C code does: `orig.cache = lfs->rcache; lfs->rcache.block = 0xffffffff;`

            // Detach lfs.rcache temporarily for 'orig' file reads
            let mut temp_rcache_for_orig = lfs.rcache; // Copy struct state
            lfs.rcache.block = 0xffffffff; // Invalidate main rcache for safety / as per C
            orig_file_for_read.cache = temp_rcache_for_orig;


            while file.pos < file.size { // While there's old data to copy past current write pos
                let mut byte_data = [0u8;1];
                // Read one byte from original file state
                match lfs_file_read(lfs, &mut orig_file_for_read, &mut byte_data) {
                    Ok(0) => break, // End of original file
                    Ok(1) => {
                        // Write this byte to the current file (advancing its new data)
                        if lfs_file_write(lfs, file, &byte_data)? != 1 {
                            // Restore lfs.rcache state if we borrowed it
                            lfs.rcache = orig_file_for_read.cache;
                            return Err(LfsError::Io); // Write error
                        }
                    }
                    Ok(_) => {
                        // More than 1 byte read, which is unexpected
                        lfs.rcache = orig_file_for_read.cache; // Restore state
                        return Err(LfsError::Io); // Read error
                    }
                    Err(e) => { lfs.rcache = orig_file_for_read.cache; return Err(e); }
                }
            }
            // Restore lfs.rcache if it was borrowed/copied.
            lfs.rcache = orig_file_for_read.cache; // or the saved state
        }

        // Write out the contents of file.cache (which acts as pcache for this file)
        // lfs_cache_flush here should use file.cache as the pcache, and lfs.rcache for verification.
        // This call might trigger lfs_file_relocate if corruption occurs.
        lfs_cache_flush(cfg, &mut file.cache, Some(&mut lfs.rcache))?;
        
        // Actual file metadata updates
        file.head = file.block; // Last written block becomes new head
        file.size = file.pos;   // Current position is new size
        file.flags &= !LfsOpenFlags::F_WRITING;
        file.flags |= LfsOpenFlags::F_DIRTY;

        file.pos = original_pos; // Restore original position
    }
    Ok(())
}

pub fn lfs_file_sync(lfs: &mut Lfs, file: &mut LfsFile) -> Result<(), LfsError> {
    lfs_file_flush(lfs, file)?;

    if (file.flags & LfsOpenFlags::F_DIRTY) != 0 && file.pair[0] != 0xffffffff {
        let mut cwd = LfsDir { /* ... zeroed ... */
             pair: [0,0], off: 0, head: [0,0], pos: 0,
             d: LfsDiskDir { rev: 0, size: 0, tail: [0,0] },
        };
        lfs_dir_fetch(lfs, &mut cwd, &file.pair)?;

        let mut entry_d_bytes = vec![0u8; core::mem::size_of::<LfsDiskEntry>()];
        lfs_bd_read(lfs, cwd.pair[0], file.poff, &mut entry_d_bytes)?;
        
        let mut entry_d = unsafe { core::ptr::read(entry_d_bytes.as_ptr() as *const LfsDiskEntry) };

        if entry_d.type_0 != LfsType::Reg as u8 {
            return Err(LfsError::Inval); // Should be a file entry
        }

        entry_d.u.file.head = file.head;
        entry_d.u.file.size = file.size;
        
        // Need to reconstruct LfsEntry to pass to lfs_dir_update
        let entry_to_update = LfsEntry { off: file.poff, d: entry_d };
        // lfs_dir_update also needs name, but here we are only updating head/size, not name.
        // The C code passes NULL for data to lfs_dir_update.
        lfs_dir_update(lfs, &mut cwd, &entry_to_update, None)?;

        file.flags &= !LfsOpenFlags::F_DIRTY;
    }
    Ok(())
}

pub fn lfs_file_read(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    buffer_out: &mut [u8],
) -> Result<LfsSize, LfsError> { // Returns bytes read
    if (file.flags & LfsOpenFlags::RDWR) == LfsOpenFlags::WRONLY { // Check if WRONLY
        return Err(LfsError::Inval);
    }

    if (file.flags & LfsOpenFlags::F_WRITING) != 0 {
        lfs_file_flush(lfs, file)?;
    }

    let max_readable = file.size.saturating_sub(file.pos);
    let actual_read_size = lfs_min(buffer_out.len() as LfsSize, max_readable) as usize;
    if actual_read_size == 0 {
        return Ok(0);
    }
    
    let mut total_bytes_read_this_call = 0;
    let mut current_buffer_slice = &mut buffer_out[..actual_read_size];

    let cfg = unsafe { &*lfs.cfg };

    while total_bytes_read_this_call < actual_read_size {
        // Check if we need a new block or if cache is at block boundary
        if (file.flags & LfsOpenFlags::F_READING) == 0 || file.off >= cfg.block_size { // C code: file->off == lfs->cfg->block_size
            match lfs_index_find(cfg, &mut file.cache, None, file.head, file.size, file.pos) {
                Ok((block, off_in_block)) => {
                    file.block = block;
                    file.off = off_in_block;
                }
                Err(e) => return Err(e),
            }
            file.flags |= LfsOpenFlags::F_READING;
            // file.cache.block might need invalidation if lfs_index_find uses it as rcache and modifies it for a *different* block.
            // But lfs_index_find takes file.cache as rcache, so it's fine.
        }

        let remaining_in_cache_block = cfg.block_size.saturating_sub(file.off);
        let bytes_to_read_now = lfs_min(current_buffer_slice.len() as LfsSize, remaining_in_cache_block) as usize;

        if file.block == 0xffffffff { return Err(LfsError::Io); } // Trying to read from invalid block

        lfs_cache_read(
            cfg,
            &mut file.cache, // file.cache acts as its own read cache here
            None,            // No pcache involved for pure read from file's blocks
            file.block,
            file.off,
            &mut current_buffer_slice[..bytes_to_read_now],
        )?;

        file.pos += bytes_to_read_now as LfsOff;
        file.off += bytes_to_read_now as LfsOff;
        total_bytes_read_this_call += bytes_to_read_now;
        current_buffer_slice = &mut current_buffer_slice[bytes_to_read_now..];
    }

    Ok(total_bytes_read_this_call as LfsSize)
}

pub fn lfs_file_write(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    buffer_in: &[u8],
) -> Result<LfsSize, LfsError> { // Returns bytes written
    if (file.flags & LfsOpenFlags::RDWR) == LfsOpenFlags::RDONLY { // Check if RDONLY
        return Err(LfsError::Inval);
    }

    if (file.flags & LfsOpenFlags::F_READING) != 0 {
        // C code calls lfs_file_flush which drops read cache.
        // If flush also syncs writes, that's fine.
        // Here, just clearing read state is enough if flush handles it.
        lfs_file_flush(lfs, file)?; // This will also clear F_READING flag
    }
    
    if (file.flags & LfsOpenFlags::APPEND) != 0 && file.pos < file.size {
        file.pos = file.size;
        // file.off might need recalculation if pos changes, or will be handled by index_find/extend.
        // Setting file.off to cfg.block_size will force a new block find/extend.
        file.off = unsafe{(&*lfs.cfg).block_size};
    }

    let mut total_bytes_written_this_call = 0;
    let mut current_buffer_slice = buffer_in;
    let cfg = unsafe { &*lfs.cfg };

    if current_buffer_slice.is_empty() {
        return Ok(0);
    }

    while total_bytes_written_this_call < buffer_in.len() {
        // Check if we need a new block or if cache is at block boundary
        if (file.flags & LfsOpenFlags::F_WRITING) == 0 || file.off >= cfg.block_size {
            if (file.flags & LfsOpenFlags::F_WRITING) == 0 {
                // Find out which block we're extending from
                match lfs_index_find(cfg, &mut file.cache, None, file.head, file.size, file.pos) {
                     Ok((block, off_in_block)) => {
                        file.block = block;
                        file.off = off_in_block;
                    }
                    Err(e) => return Err(e),
                }
                // Mark file.cache as dirty since we may have read into it if lfs_index_find used it.
                // More importantly, it's now going to be used as a program cache.
                file.cache.block = 0xffffffff;
                file.flags |= LfsOpenFlags::F_WRITING;
            }

            // Extend file with new blocks
            lfs_alloc_ack(lfs);
            // lfs_index_extend needs lfs, rcache (global), pcache (file.cache)
            match lfs_index_extend(lfs, &mut lfs.rcache.clone(), &mut file.cache, file.block, file.pos /* or file.size before this write? C: file.pos */) {
                Ok((new_block, off_in_new_block)) => {
                    file.block = new_block;
                    file.off = off_in_new_block;
                }
                Err(e) => return Err(e),
            }
        }
        
        let remaining_in_cache_block = cfg.block_size.saturating_sub(file.off);
        let bytes_to_write_now = lfs_min(current_buffer_slice.len() as LfsSize, remaining_in_cache_block) as usize;

        'relocate_prog: loop { // Loop for relocate on prog corruption
            match lfs_cache_prog(
                cfg,
                &mut file.cache,    // file.cache is the pcache for this file's writes
                Some(&mut lfs.rcache), // lfs.rcache for verification if prog bypasses file.cache
                file.block,
                file.off,
                &current_buffer_slice[..bytes_to_write_now],
            ) {
                Ok(_) => break 'relocate_prog,
                Err(LfsError::Corrupt) => {
                    if lfs_file_relocate(lfs, file).is_err() { // If relocate itself fails
                        return Err(LfsError::Corrupt); // Or the error from relocate
                    }
                    // Relocate successful, retry prog in the next iteration of 'relocate_prog
                }
                Err(e) => return Err(e),
            }
        }


        file.pos += bytes_to_write_now as LfsOff;
        file.off += bytes_to_write_now as LfsOff;
        total_bytes_written_this_call += bytes_to_write_now;
        current_buffer_slice = &current_buffer_slice[bytes_to_write_now..];

        lfs_alloc_ack(lfs); // Acknowledge blocks used by lfs_index_extend
    }

    // Mark file as dirty, but actual metadata (size, head) update happens in flush/sync
    file.flags |= LfsOpenFlags::F_DIRTY; 
    // C code sets F_DIRTY in lfs_file_flush, not here directly.
    // But writing definitely makes it dirty wrt to its entry on disk.
    // Let's follow C: F_DIRTY is set by lfs_file_flush when F_WRITING is cleared.
    
    Ok(total_bytes_written_this_call as LfsSize)
}

pub fn lfs_file_seek(
    lfs: &mut Lfs,
    file: &mut LfsFile,
    offset: LfsSOff,
    whence: LfsWhenceFlags,
) -> Result<LfsOff, LfsError> { // Returns old position as LfsOff
    // Write out any pending changes before seeking
    lfs_file_flush(lfs, file)?;

    let old_pos = file.pos;

    match whence {
        LfsWhenceFlags::Set => {
            if offset < 0 {
                return Err(LfsError::Inval); // Negative absolute offset is invalid
            }
            file.pos = offset as LfsOff;
        }
        LfsWhenceFlags::Cur => {
            // Check for overflow/underflow before applying
            if offset > 0 {
                file.pos = file.pos.saturating_add(offset as LfsOff);
            } else if offset < 0 {
                file.pos = file.pos.saturating_sub(offset.unsigned_abs() as LfsOff);
            }
            // If file.pos became very large due to offset, it's okay; reads/writes will be bounded by file.size.
            // LittleFS C code just adds, potential for wrapping if not careful with LfsSOff range.
            // Rust's LfsOff is u32. If offset makes it negative conceptually, it should go towards 0.
            // Correct logic for CUR:
            // let new_pos = (file.pos as LfsSOff).wrapping_add(offset);
            // if new_pos < 0 {
            //     return Err(LfsError::Inval); // Seeking before start of file
            // }
            // file.pos = new_pos as LfsOff;
            // The C code does: file->pos = file->pos + off;
            // If file.pos is LfsOff (u32) and off is LfsSOff (i32):
            let base_pos = file.pos as i64; // Use i64 to avoid overflow during intermediate calc
            let new_pos_candidate = base_pos + (offset as i64);
            if new_pos_candidate < 0 {
                return Err(LfsError::Inval); // Seeking to before the beginning of the file
            }
            file.pos = new_pos_candidate as LfsOff;
        }
        LfsWhenceFlags::End => {
            // Similar to CUR, use i64 for intermediate calculation
            let base_pos = file.size as i64;
            let new_pos_candidate = base_pos + (offset as i64);
            if new_pos_candidate < 0 {
                return Err(LfsError::Inval); // Seeking to before the beginning of the file (relative to end)
            }
            file.pos = new_pos_candidate as LfsOff;
        }
        // _ => return Err(LfsError::Inval), // Should not happen if LfsWhenceFlags is exhaustive
    }

    // Note: Unlike C's fseek, lfs_file_seek does not necessarily restrict
    // file.pos to be within file.size here. Reads/writes will handle boundaries.
    // The return value is the *old* position.
    Ok(old_pos)
}

pub fn lfs_file_tell(_lfs: &Lfs, file: &LfsFile) -> LfsSOff {
    // In C, this can return a negative error code if file is invalid, but here,
    // we assume 'file' is valid if we have a reference. Errors are for operations.
    // If file.pos (LfsOff/u32) can exceed LfsSOff/i32 max, this cast could truncate.
    // However, LfsSOff is i32, LfsOff is u32. It's usually fine if pos is within i32::MAX.
    if file.pos > i32::MAX as u32 {
        // Position is too large to be represented by LfsSOff.
        // The C API for tell often returns -1 on error.
        // Here, we might return a large positive that indicates this, or error.
        // For now, direct cast as per C.
        // Consider if LfsError should be returned or if LfsSOff can represent all valid LfsOff.
        // Given LfsSOff is used for seek offsets too, file.pos should ideally fit.
    }
    file.pos as LfsSOff
}

pub fn lfs_file_rewind(lfs: &mut Lfs, file: &mut LfsFile) -> Result<(), LfsError> {
    match lfs_file_seek(lfs, file, 0, LfsWhenceFlags::Set) {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn lfs_file_size(_lfs: &Lfs, file: &LfsFile) -> LfsSOff {
    // file.pos could be beyond file.size if O_APPEND writes occurred but not yet synced
    // to update file.size from the directory entry, or if seeked past EOF.
    // lfs_max used in C.
    let effective_size = lfs_max(file.pos, file.size);
    // Similar to lfs_file_tell, consider if effective_size (LfsOff/u32) can exceed LfsSOff/i32 max.
    effective_size as LfsSOff
}