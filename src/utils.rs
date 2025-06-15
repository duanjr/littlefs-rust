use super::defines::*;

#[inline]
pub fn lfs_max(a: u32, b: u32) -> u32 {
    if a > b { a } else { b }
}

#[inline]
pub fn lfs_min(a: u32, b: u32) -> u32 {
    if a < b { a } else { b }
}

#[inline]
pub fn lfs_ctz(a: u32) -> u32 {
    a.trailing_zeros()
}

#[inline]
pub fn lfs_npw2(a: u32) -> u32 {

    if a == 0 {
        u32::BITS - (u32::MAX).leading_zeros() // effectively u32::BITS or 32
    } else {
        u32::BITS - (a - 1).leading_zeros()
    }
}


#[inline]
pub fn lfs_scmp(a: u32, b: u32) -> i32 {
    // Simulates C's `(int)(unsigned)(a - b)` for comparing sequence numbers
    a.wrapping_sub(b) as i32
}

pub fn lfs_crc(crc: &mut u32, buffer: &[u8]) {
    const RTABLE: [u32; 16] = [
        0x00000000, 0x1db71064, 0x3b6e20c8, 0x26d930ac,
        0x76dc4190, 0x6b6b51f4, 0x4db26158, 0x5005713c,
        0xedb88320, 0xf00f9344, 0xd6d6a3e8, 0xcb61b38c,
        0x9b64c2b0, 0x86d3d2d4, 0xa00ae278, 0xbdbdf21c,
    ];

    for &byte_val in buffer {
        *crc = (*crc >> 4) ^ RTABLE[((*crc ^ (byte_val >> 0) as u32) & 0xf) as usize];
        *crc = (*crc >> 4) ^ RTABLE[((*crc ^ (byte_val >> 4) as u32) & 0xf) as usize];
    }
}

#[inline]
pub fn lfs_pairswap(pair: &mut [LfsBlock; 2]) {
    pair.swap(0, 1);
}

#[inline]
pub fn lfs_pairisnull(pair: &[LfsBlock; 2]) -> bool {
    pair[0] == 0xffffffff || pair[1] == 0xffffffff
}

#[inline]
pub fn lfs_paircmp(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> bool {
    // Original C returns 0 if they "match" (share any block or are mirrors), 1 otherwise.
    // This means it returns true (1) if they are distinct/different, false (0) if they match/overlap.
    !((paira[0] == pairb[0] || paira[0] == pairb[1]) ||
      (paira[1] == pairb[0] || paira[1] == pairb[1]))
}

#[inline]
pub fn lfs_pairsync(paira: &[LfsBlock; 2], pairb: &[LfsBlock; 2]) -> bool {
    (paira[0] == pairb[0] && paira[1] == pairb[1]) ||
    (paira[0] == pairb[1] && paira[1] == pairb[0])
}