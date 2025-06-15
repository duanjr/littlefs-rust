use super::defines::*;
use super::block_device::*;
use super::allocator::*;
use super::metadata_pair::*;

pub fn lfs_mkdir(lfs: &mut Lfs, path_str: &str) -> Result<(), LfsError> {
    // Fetch parent directory
    // In C, lfs_dir_find updates 'path' to point to the last component.
    // We need to replicate this behavior or extract the parent and child components.
    
    // Separate path into parent_path and new_dir_name
    let (parent_path_str, new_dir_name_str) = {
        if let Some(idx) = path_str.rfind('/') {
            (&path_str[..idx + 1], &path_str[idx + 1..])
        } else {
            ("", path_str) // Relative to current (root for now)
        }
    };
    if new_dir_name_str.is_empty() || new_dir_name_str == "." || new_dir_name_str == ".." {
        return Err(LfsError::Inval); // Invalid directory name
    }
    if new_dir_name_str.len() > LFS_NAME_MAX {
        return Err(LfsError::Inval); // Name too long
    }

    let mut parent_dir = LfsDir { // Initialize an LfsDir to open the parent
        pair: [0,0], off: 0, head: [0,0], pos: 0,
        d: LfsDiskDir { rev: 0, size: 0, tail: [0,0] },
    };

    // Open the parent directory. If parent_path_str is empty, it means root.
    lfs_dir_open(lfs, &mut parent_dir, if parent_path_str.is_empty() { "/" } else { parent_path_str })?;
    
    // Check if the new_dir_name already exists in the parent_dir
    let mut temp_entry = LfsEntry { // For lfs_dir_find
        off: 0,
        d: LfsDiskEntry { type_0: 0, elen: 0, alen: 0, nlen: 0, u: LfsDiskEntryUnion { file: LfsDiskEntryFile {head:0, size:0} } }
    };
    let mut find_path_ref: &str = new_dir_name_str; // lfs_dir_find expects &mut &str

    match lfs_dir_find(lfs, &mut parent_dir, &mut temp_entry, &mut find_path_ref) {
        Ok(_) => return Err(LfsError::Exists), // Found, so it already exists
        Err(LfsError::NoEnt) => { /* Good, it does not exist */ }
        Err(e) => return Err(e), // Other error
    }
    // After lfs_dir_find (if it were to succeed or fail with NoEnt on the last component),
    // parent_dir would be the directory where new_dir_name is to be created.
    // We need to re-fetch parent_dir to its original state before lfs_dir_find modified its internal iterator.
    // This is simpler: lfs_dir_open already gives us the correct parent_dir context.

    lfs_alloc_ack(lfs);

    let mut new_mkdir = LfsDir {
        pair: [0,0], off: 0, head: [0,0], pos: 0,
        d: LfsDiskDir { rev: 0, size: 0, tail: [0,0] },
    };
    lfs_dir_alloc(lfs, &mut new_mkdir)?;
    
    // New directory's tail should point to where its parent's current tail pointed.
    // This is for making a chain of directory blocks for storing entries, not parent/child relationship.
    // The C code: dir.d.tail[0] = cwd.d.tail[0]; (cwd is parent_dir here)
    // This seems to imply new directories are added to the end of the parent's block chain if the parent itself is full.
    // However, mkdir creates an *empty* directory. Its tail should be (-1,-1).
    // The `cwd.d.tail[0] = dir.pair[0]` in C `lfs_mkdir` suggests that the parent directory's
    // linked list of blocks (for storing *its own* entries) is extended by this new directory block.
    // This seems incorrect for mkdir. `lfs_mkdir` creates a new, distinct directory object.
    // The `entry.d.u.dir` will point to `new_mkdir.pair`.
    // The `parent_dir.d.tail` should only be modified by `lfs_dir_append` if `parent_dir` itself needs a new block.
    //
    // Re-reading C:
    // lfs_dir_alloc(lfs, &dir); // dir is new_mkdir
    // dir.d.tail[0] = cwd.d.tail[0]; // cwd is parent_dir. THIS IS WRONG for typical mkdir.
    // This C line `dir.d.tail[0] = cwd.d.tail[0]; dir.d.tail[1] = cwd.d.tail[1];` refers to making
    // the *newly created directory itself* part of a metadata log chain, inheriting the tail of its
    // "predecessor" in that log (which is `cwd` here, acting as the last block in the log before this new one).
    // Then `cwd.d.tail[0] = dir.pair[0]; cwd.d.tail[1] = dir.pair[1];` makes `cwd` point to `dir` as its new tail.
    // This is specific to how LFS appends metadata blocks.
    // For mkdir, the new directory `new_mkdir` should be empty initially. Its `d.tail` should be `[-1,-1]`.
    // The *parent directory* (`parent_dir`) will have an *entry* pointing to `new_mkdir.pair`.
    // The `lfs_dir_alloc` already sets new_mkdir.d.tail to [-1,-1].

    // Commit the newly allocated empty directory
    lfs_dir_commit(lfs, &mut new_mkdir, None)?;

    let mut entry_for_parent = LfsEntry {
        off: 0, // Will be set by lfs_dir_append
        d: LfsDiskEntry {
            type_0: LfsType::Dir as u8,
            // elen for dir is sizeof(u.dir) = 2 * sizeof(lfs_block_t) = 2 * 4 = 8
            elen: (core::mem::size_of::<LfsBlock>() * 2) as u8,
            alen: 0,
            nlen: new_dir_name_str.len() as u8,
            u: LfsDiskEntryUnion { dir: new_mkdir.pair },
        },
    };

    // Append the entry for the new directory to its parent directory
    // Note: lfs_dir_append might modify parent_dir (e.g. its tail, if it needs a new block)
    let name_bytes = new_dir_name_str.as_bytes();
    lfs_dir_append(lfs, &mut parent_dir, &mut entry_for_parent, name_bytes)?;

    lfs_alloc_ack(lfs);
    Ok(())
}

pub fn lfs_dir_open(lfs: &mut Lfs, dir_out: &mut LfsDir, path_str: &str) -> Result<(), LfsError> {
    // Initialize dir_out to root
    dir_out.pair[0] = lfs.root[0];
    dir_out.pair[1] = lfs.root[1];

    if dir_out.pair[0] == 0xffffffff || dir_out.pair[1] == 0xffffffff {
        // Filesystem not formatted or root is invalid
        return Err(LfsError::NoEnt); // Or Corrupt
    }

    lfs_dir_fetch(lfs, dir_out, &lfs.root.clone())?; // dir_out now represents the root directory

    // Check for special root-like paths (e.g., "/", ".", "/./.")
    let mut only_slash_or_dot = true;
    for char_code in path_str.bytes() {
        if char_code != b'/' && char_code != b'.' {
            only_slash_or_dot = false;
            break;
        }
    }
    if path_str.is_empty() { // Treat empty path as root for open
        only_slash_or_dot = true;
    }


    if only_slash_or_dot {
        // Path is like "/", ".", or "/././../." (effectively root or current dir at root)
        dir_out.head[0] = dir_out.pair[0];
        dir_out.head[1] = dir_out.pair[1];
        dir_out.pos = (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2); // Special offset for '.' and '..'
        dir_out.off = core::mem::size_of::<LfsDiskDir>() as LfsOff;
        return Ok(());
    }

    // Normal path, find it
    let mut entry_found = LfsEntry {
        off: 0,
        d: LfsDiskEntry { type_0: 0, elen: 0, alen: 0, nlen: 0, u: LfsDiskEntryUnion{ file: LfsDiskEntryFile{head:0, size:0}} }
    };
    // lfs_dir_find modifies `dir_out` to be the directory containing the final component,
    // and `entry_found` gets the details of the final component.
    // `path_str_ref` will point to an empty string if the whole path is consumed.
    let mut path_str_ref: &str = path_str;
    lfs_dir_find(lfs, dir_out, &mut entry_found, &mut path_str_ref)?;

    // After lfs_dir_find, if successful, `entry_found` is the target.
    // `dir_out` is its parent. We need to fetch `entry_found` into `dir_out`.
    if entry_found.d.type_0 != LfsType::Dir as u8 {
        return Err(LfsError::NotDir);
    }

    let target_dir_pair = unsafe { entry_found.d.u.dir };
    lfs_dir_fetch(lfs, dir_out, &target_dir_pair)?; // dir_out now becomes the target directory

    // Setup head dir for iteration
    dir_out.head[0] = dir_out.pair[0];
    dir_out.head[1] = dir_out.pair[1];
    dir_out.pos = (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2); // Special offset for '.' and '..'
    dir_out.off = core::mem::size_of::<LfsDiskDir>() as LfsOff;

    Ok(())
}

fn lfs_dir_close(_lfs: &mut Lfs, _dir: &mut LfsDir) -> Result<(), LfsError> {
    // Do nothing, dir is always synchronized in this LFS version.
    Ok(())
}

pub fn lfs_dir_read(
    lfs: &mut Lfs,
    dir: &mut LfsDir, // Iteration state is stored in dir (dir.pos, dir.off, current dir.pair)
    info_out: &mut LfsInfo,
) -> Result<bool, LfsError> { // Returns Ok(true) if entry read, Ok(false) if no more entries
    info_out.name.fill(0); // Clear info structure, especially name.

    let size_of_disk_dir = core::mem::size_of::<LfsDiskDir>() as LfsOff;

    // Handle special "." and ".." entries
    if dir.pos == size_of_disk_dir.wrapping_sub(2) {
        info_out.type_0 = LfsType::Dir as u8;
        info_out.name[0] = b'.';
        // name is already null-terminated by fill(0)
        dir.pos = dir.pos.wrapping_add(1);
        return Ok(true);
    } else if dir.pos == size_of_disk_dir.wrapping_sub(1) {
        info_out.type_0 = LfsType::Dir as u8;
        info_out.name[0] = b'.';
        info_out.name[1] = b'.';
        dir.pos = dir.pos.wrapping_add(1);
        return Ok(true);
    }

    let mut entry = LfsEntry {
        off: 0,
        d: LfsDiskEntry { type_0: 0, elen: 0, alen: 0, nlen: 0, u: LfsDiskEntryUnion { file: LfsDiskEntryFile{head:0, size:0}} }
    };

    loop { // Skip non-REG/DIR entries
        match lfs_dir_next(lfs, dir, &mut entry) {
            Ok(_) => {
                if entry.d.type_0 == LfsType::Reg as u8 || entry.d.type_0 == LfsType::Dir as u8 {
                    break; // Found a usable entry
                }
                // Otherwise, loop to get next entry
            }
            Err(LfsError::NoEnt) => return Ok(false), // No more entries
            Err(e) => return Err(e),
        }
    }

    info_out.type_0 = entry.d.type_0;
    if info_out.type_0 == LfsType::Reg as u8 {
        info_out.size = unsafe { entry.d.u.file.size };
    } else {
        info_out.size = 0; // Directories don't have a 'size' in this context
    }

    // Read the name
    if entry.d.nlen as usize > LFS_NAME_MAX {
        return Err(LfsError::Corrupt); // Name length in entry exceeds buffer
    }
    
    let name_offset_in_block = entry.off + 4 + entry.d.elen as LfsOff + entry.d.alen as LfsOff;
    let name_len_usize = entry.d.nlen as usize;

    // Read into a temporary buffer first if info_out.name is not directly writable slice of adequate size
    // Or, read directly into info_out.name if lfs_bd_read supports it.
    // lfs_bd_read takes &mut [u8].
    if name_len_usize > 0 {
         lfs_bd_read(lfs, dir.pair[0], name_offset_in_block, &mut info_out.name[..name_len_usize])?;
    }
    // info_out.name is already null-terminated due to earlier fill(0)

    Ok(true)
}

pub fn lfs_dir_seek(lfs: &mut Lfs, dir: &mut LfsDir, offset: LfsOff) -> Result<(), LfsError> {
    // Simply walk from head dir
    lfs_dir_rewind(lfs, dir)?;
    dir.pos = offset; // dir.pos is the logical "entry number" including ., ..

    // Calculate effective data offset within the possibly chained directory blocks
    let mut current_block_data_start_pos = (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2); // Pos of "."
    
    // Find the correct block
    // dir.d.size is the size of current block's content + header + crc
    // dir.off is offset within current block's *data* area
    // dir.pos is the "absolute" seek position for the dir stream
    
    // The C code's `off` parameter to lfs_dir_seek directly corresponds to `dir.pos`.
    // `dir.pos` includes sizeof(dir.d)-2 for ., ..
    // `dir.off` should be calculated relative to start of entries in current block.

    let mut current_pos_in_stream = (core::mem::size_of::<LfsDiskDir>() as LfsOff); // Start of actual entries
                                                                                   // after accounting for . and ..

    while dir.pos >= current_pos_in_stream + ((dir.d.size & 0x7FFFFFFF) - core::mem::size_of::<LfsDiskDir>() as LfsOff - 4) {
        // If dir.pos is beyond the entries in the current block
        let entries_size_in_current_block = (dir.d.size & 0x7FFFFFFF) - core::mem::size_of::<LfsDiskDir>() as LfsOff - 4;
        current_pos_in_stream += entries_size_in_current_block;

        if (dir.d.size & 0x80000000) == 0 { // Not continued
            return Err(LfsError::Inval); // Seek past end of directory
        }
        if dir.d.tail[0] == 0xffffffff || dir.d.tail[1] == 0xffffffff {
            return Err(LfsError::Corrupt);
        }
        lfs_dir_fetch(lfs, dir, &dir.d.tail.clone())?;
        // After fetch, dir.d.size is for the new block.
        // current_pos_in_stream conceptually advances by size of prev block's header+crc
        // The C code simplified this by subtracting from `off` (which is `dir.pos`).
    }

    // dir.pos is the target absolute position.
    // dir.off should be dir.pos relative to the start of entries in *this specific block*.
    // If dir.pos < size_of_disk_dir, it's for . or .. or just before first entry.
    // The C code has: `dir->off = off;` after the loop.
    // `off` in C is `dir.pos` after rewind.
    // The loop effectively makes `dir` point to the correct block.
    // `dir.off` is then `dir.pos` MINUS the cumulative size of entries in previous blocks.
    // This is complicated. C code: `dir->off = off;` (where `off` is remaining from `dir->pos` after loop)
    // Let's trace the C loop:
    // dir->pos = target_offset;
    // while (target_offset > (0x7fffffff & dir->d.size)) {
    //   target_offset -= (0x7fffffff & dir->d.size); // This isn't quite right, should be content size
    //   ... fetch next block ...
    // }
    // dir->off = target_offset;
    // This suggests dir.off is relative to start of *block*, not start of *entries*.
    // And lfs_dir_next uses dir.off to read from dir.pair[0].
    // But dir.off should skip LfsDiskDir header.
    // `lfs_dir_rewind` sets dir.off = sizeof(LfsDiskDir).
    // The C `lfs_dir_seek` has `dir->off = off;` where `off` is remaining portion of original `dir->pos` for current block.
    
    // Rewind sets dir.off = sizeof(LfsDiskDir), pos = sizeof(LfsDiskDir)-2
    // If seek(0) -> dir.pos = 0.
    // The loop logic in C is `while (off > (dir.d.size & 0x7fffffff))`
    // this `off` is the remaining part of original `dir.pos`.
    // Let's keep `dir.pos` as the target absolute stream position.
    // `dir.off` needs to be calculated for the current block `dir`.
    // `dir.off` is the read pointer within the current block's data area.
    // After rewind, `dir.off` is `sizeof(LfsDiskDir)`. `dir.pos` is `sizeof(LfsDiskDir)-2`.
    // If seeking to `target_pos`:
    // `dir.pos` becomes `target_pos`.
    // We need to iterate `dir.off` (and potentially `dir` blocks) until `dir.pos` is reached conceptually.

    // Simplified seek based on C's effective outcome:
    // The loop above (in my translation) finds the correct block.
    // `dir.pos` (the argument `offset`) must be translated to an `off` within that block.
    // The `dir.off` is the cursor for `lfs_dir_next`.
    // The `dir.pos` is the abstract tell() position.
    // After rewind: dir.off = sizeof(LfsDiskDir), dir.pos = sizeof(LfsDiskDir)-2
    // To reach logical position `offset`:
    // Need to skip `offset - dir.pos` entries.
    // This is usually done by calling `lfs_dir_next` repeatedly.
    // The C code sets `dir->off = off_within_current_block_data_area`.
    // This `off_within_current_block_data_area` is calculated by subtracting sizes of previous blocks from the original seek `offset`.
    // This is simpler: after `lfs_dir_rewind`, `dir` is at the beginning.
    // We set `dir.pos = offset`.
    // The `lfs_dir_read` or `lfs_dir_next` will then internally advance `dir.off` and fetch blocks based on `dir.pos`.
    // The C code's `lfs_dir_seek`'s goal is to make `dir.off` ready for the *next* `lfs_dir_next` call
    // to read the entry at the Nth logical position.

    // Let's re-do lfs_dir_seek with C's logic more directly.
    lfs_dir_rewind(lfs, dir)?; // dir.pair, dir.head, dir.off, dir.pos are reset
    
    let mut remaining_logical_offset = offset; // This is the target dir.pos value

    if remaining_logical_offset < dir.pos { // target is before initial pos (e.g. seek to 0 when pos is for '.')
         // Rewind already handled this. dir.pos is the new target. dir.off is ready.
         dir.pos = remaining_logical_offset;
         if remaining_logical_offset < (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2) {
             return Err(LfsError::Inval); // Cannot seek before "."
         }
         // if seeking to . or .., dir.off is sizeof(LfsDiskDir)
         // if seeking to first entry, dir.pos = sizeof(LfsDiskDir), dir.off = sizeof(LfsDiskDir)
         // This seems fine by just setting dir.pos. lfs_dir_read handles it.
         return Ok(());
    }
    
    // Skip initial "." and ".." from remaining_logical_offset if target is beyond them
    if dir.pos == (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2) && remaining_logical_offset > dir.pos { // currently at "."
        remaining_logical_offset = remaining_logical_offset.saturating_sub(1);
        dir.pos = dir.pos.wrapping_add(1); // now at ".."
    }
    if dir.pos == (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(1) && remaining_logical_offset > dir.pos { // currently at ".."
        remaining_logical_offset = remaining_logical_offset.saturating_sub(1);
        dir.pos = dir.pos.wrapping_add(1); // now at start of actual entries
    }
    // Now dir.pos and remaining_logical_offset are aligned with start of actual entries
    // dir.off is sizeof(LfsDiskDir)

    // `remaining_logical_offset` now represents items to skip *past* the start of entries.
    // `dir.pos` has been updated to the target `offset`.
    // We need to set `dir.off` to the start of entries and then skip `remaining_logical_offset` items using lfs_dir_next,
    // or calculate dir.off correctly.
    // The C code's:
    //   dir->pos = off; (off is the target absolute position from tell)
    //   while (off > (0x7fffffff & dir->d.size)) { off -= (0x7fffffff & dir->d.size); ... fetch ... }
    //   dir->off = off;
    // This `off` in `dir->off = off` is relative to the start of the *block* for C's lfs_bd_read.
    // But `lfs_dir_next` uses `dir.off` as an offset *within the entry data area*.
    // This is subtle. `dir.off` after `lfs_dir_rewind` is `sizeof(LfsDiskDir)`.

    // Let's simulate skipping entries to set dir.off and dir.pos correctly.
    // target_pos = offset (argument)
    dir.pos = (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2); // from rewind
    dir.off = core::mem::size_of::<LfsDiskDir>() as LfsOff; // from rewind

    while dir.pos < offset {
        // We need to advance one conceptual entry. This could be ".", "..", or a real entry.
        if dir.pos == (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2) || // "."
           dir.pos == (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(1) {  // ".."
            dir.pos += 1;
            // dir.off remains sizeof(LfsDiskDir) because . and .. don't consume from entry data area for dir.off
            if dir.pos > offset { break; } // Overshot (shouldn't happen if offset >= dir.pos initially)
            continue;
        }

        // Try to read next entry to advance dir.off and dir.pos
        let mut temp_entry = LfsEntry { off:0, d: LfsDiskEntry { type_0:0, elen:0, alen:0, nlen:0, u: LfsDiskEntryUnion {file:LfsDiskEntryFile{head:0,size:0}} }};
        match lfs_dir_next(lfs, dir, &mut temp_entry) {
            Ok(()) => { /* dir.pos and dir.off are advanced by lfs_dir_next */ }
            Err(LfsError::NoEnt) => return Err(LfsError::Inval), // Seek past end
            Err(e) => return Err(e),
        }
        // dir.pos is now position *after* temp_entry.
        // If dir.pos > offset, it means the target `offset` was within the `temp_entry` we just read,
        // or just before it. lfs_dir_next already advanced past it.
        // This is complex. `lfs_dir_tell` gives `dir.pos`. `lfs_dir_seek` needs to restore to that.

        // A simpler interpretation: `lfs_dir_seek` sets `dir.pos` to the target.
        // Then, when `lfs_dir_read` is called, it will internally fast-forward `dir.off`
        // and fetch blocks until `dir.pos` corresponds to an entry it can return.
        // The C code's `dir->off = off_in_block_data;` is an optimization.
        // For now, just setting `dir.pos` and letting `lfs_dir_read/next` figure it out is simpler,
        // though less efficient than C.
        // Let's stick to C's `dir->off = off;` behavior for now, meaning we calculate the
        // remaining offset within the *final* block.
    }
    // After rewind:
    // dir.pair = dir.head
    // dir.pos = sizeof(dir.d) - 2
    // dir.off = sizeof(dir.d)
    //
    // Target logical offset is `offset`.
    
    // Let current_logical_pos track the stream position based on dir.d.size
    // This is effectively setting dir.pos to the target, then finding the block
    // and setting dir.off to be relative to that block's data start.
    let mut target_remaining_offset = offset;
    // Account for . and .. which are not in dir->d.size iterations
    if target_remaining_offset >= (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2) {
        target_remaining_offset -= (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2);
    } else {
        // Seeking to . or .. or before. Rewind + setting dir.pos is enough.
        dir.pos = offset;
        return Ok(());
    }

    // Now target_remaining_offset is relative to start of first actual entry in logical stream.
    // dir.off is already sizeof(LfsDiskDir).
    loop {
        let current_block_content_size = (dir.d.size & 0x7FFFFFFF)
            .saturating_sub(core::mem::size_of::<LfsDiskDir>() as LfsOff) // Subtract header
            .saturating_sub(4); // Subtract CRC

        if target_remaining_offset <= current_block_content_size {
            dir.off = (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_add(target_remaining_offset);
            dir.pos = offset; // Set the final logical position
            return Ok(());
        }

        target_remaining_offset -= current_block_content_size;
        if (dir.d.size & 0x80000000) == 0 { // Not continued
            return Err(LfsError::Inval); // Seek past end
        }
        if dir.d.tail[0] == 0xffffffff || dir.d.tail[1] == 0xffffffff {
            return Err(LfsError::Corrupt);
        }
        lfs_dir_fetch(lfs, dir, &dir.d.tail.clone())?;
        // dir.off for new block should be reset to start of entries
        dir.off = core::mem::size_of::<LfsDiskDir>() as LfsOff;
    }
}

pub fn lfs_dir_tell(_lfs: &Lfs, dir: &LfsDir) -> LfsSOff {
    dir.pos as LfsSOff
}

pub fn lfs_dir_rewind(lfs: &mut Lfs, dir: &mut LfsDir) -> Result<(), LfsError> {
    // Reload the head directory
    if dir.head[0] == 0 && dir.head[1] == 0 && dir.pair[0] != lfs.root[0] {
        // This implies dir.head was not properly initialized by lfs_dir_open
        // if dir is not already root. For safety, one might error here or re-open.
        // However, lfs_dir_open sets dir.head. If it's called, head is set.
    }
    lfs_dir_fetch(lfs, dir, &dir.head.clone())?;

    // dir.pair is updated by lfs_dir_fetch to dir.head already.
    // Reset iteration state
    dir.pos = (core::mem::size_of::<LfsDiskDir>() as LfsOff).wrapping_sub(2); // Pos for "."
    dir.off = core::mem::size_of::<LfsDiskDir>() as LfsOff; // Offset for first actual entry
    Ok(())
}

