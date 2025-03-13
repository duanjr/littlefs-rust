
/// CRC32计算函数，基于多项式0x04c11db7
/// 
/// * `crc`: 初始CRC值
/// * `buffer`: 要计算的数据缓冲区
/// * `size`: 数据长度
pub fn lfs_crc(mut crc: u32, buffer: &[u8]) -> u32 {
    // CRC表
    const RTABLE: [u32; 16] = [
        0x00000000, 0x1db71064, 0x3b6e20c8, 0x26d930ac,
        0x76dc4190, 0x6b6b51f4, 0x4db26158, 0x5005713c,
        0xedb88320, 0xf00f9344, 0xd6d6a3e8, 0xcb61b38c,
        0x9b64c2b0, 0x86d3d2d4, 0xa00ae278, 0xbdbdf21c,
    ];

    for &byte in buffer {
        crc = (crc >> 4) ^ RTABLE[((crc ^ (byte as u32 >> 0)) & 0xf) as usize];
        crc = (crc >> 4) ^ RTABLE[((crc ^ (byte as u32 >> 4)) & 0xf) as usize];
    }

    crc
}

/// 返回两个无符号32位数中的最大值
pub fn lfs_max(a: u32, b: u32) -> u32 {
    if a > b { a } else { b }
}

/// 返回两个无符号32位数中的最小值
pub fn lfs_min(a: u32, b: u32) -> u32 {
    if a < b { a } else { b }
}

/// 向下对齐到最接近的alignment的倍数
pub fn lfs_aligndown(a: u32, alignment: u32) -> u32 {
    a - (a % alignment)
}

/// 向上对齐到最接近的alignment的倍数
pub fn lfs_alignup(a: u32, alignment: u32) -> u32 {
    lfs_aligndown(a + alignment - 1, alignment)
}

/// 找到大于或等于a的最小2的幂
pub fn lfs_npw2(a: u32) -> u32 {
    let mut r = 0;
    let mut a = a - 1;
    
    let s = ((a > 0xffff) as u32) << 4; 
    a >>= s; 
    r |= s;
    
    let s = ((a > 0xff) as u32) << 3; 
    a >>= s; 
    r |= s;
    
    let s = ((a > 0xf) as u32) << 2; 
    a >>= s; 
    r |= s;
    
    let s = ((a > 0x3) as u32) << 1; 
    a >>= s; 
    r |= s;
    
    (r | (a >> 1)) + 1
}

/// 计算a中尾随二进制零的数量
/// 注意：lfs_ctz(0)可能未定义
pub fn lfs_ctz(a: u32) -> u32 {
    lfs_npw2((a & a.wrapping_neg()) + 1) - 1
}

/// 计算a中二进制1的数量
pub fn lfs_popc(a: u32) -> u32 {
    let mut a = a;
    a = a - ((a >> 1) & 0x55555555);
    a = (a & 0x33333333) + ((a >> 2) & 0x33333333);
    (((a + (a >> 4)) & 0xf0f0f0f) * 0x1010101) >> 24
}

/// 查找a和b的序列比较，这是a和b之间的距离，忽略溢出
pub fn lfs_scmp(a: u32, b: u32) -> i32 {
    (a.wrapping_sub(b)) as i32
}

/// 在32位小端字节序和本机字节序之间转换
pub fn lfs_fromle32(a: u32) -> u32 {
    if cfg!(target_endian = "little") {
        a
    } else {
        a.swap_bytes()
    }
}

/// 在本机字节序和32位小端字节序之间转换
pub fn lfs_tole32(a: u32) -> u32 {
    lfs_fromle32(a)
}

/// 在32位大端字节序和本机字节序之间转换
pub fn lfs_frombe32(a: u32) -> u32 {
    if cfg!(target_endian = "big") {
        a
    } else {
        a.swap_bytes()
    }
}

/// 在本机字节序和32位大端字节序之间转换
pub fn lfs_tobe32(a: u32) -> u32 {
    lfs_frombe32(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let crc = lfs_crc(0, &data);
        assert_ne!(crc, 0); // 简单检查CRC不为0
    }

    #[test]
    fn test_align() {
        assert_eq!(lfs_aligndown(10, 4), 8);
        assert_eq!(lfs_alignup(10, 4), 12);
    }

    #[test]
    fn test_npw2() {
        assert_eq!(lfs_npw2(10), 16);
        assert_eq!(lfs_npw2(16), 16);
        assert_eq!(lfs_npw2(17), 32);
    }
}
