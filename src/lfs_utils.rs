/*
 * lfs util functions
 */

extern crate alloc;

#[cfg(not(feature = "lfs_no_malloc"))]
use alloc::alloc::{alloc, dealloc, Layout};

#[macro_export]
macro_rules! LFS_TRACE {
    ($($arg:tt)*) => {
        #[cfg(feature = "lfs_trace")]
        {
            use core::fmt::Write;
            let _ = write!($crate::util::LogWriter, "{}:{}:trace: ", file!(), line!());
            let _ = writeln!($crate::util::LogWriter, $($arg)*);
        }
    };
}

#[macro_export]
macro_rules! LFS_DEBUG {
    ($($arg:tt)*) => {
        #[cfg(not(feature = "lfs_no_debug"))]
        {
            use core::fmt::Write;
            let _ = write!($crate::util::LogWriter, "{}:{}:debug: ", file!(), line!());
            let _ = writeln!($crate::util::LogWriter, $($arg)*);
        }
    };
}

#[macro_export]
macro_rules! LFS_WARN {
    ($($arg:tt)*) => {
        #[cfg(not(feature = "lfs_no_warn"))]
        {
            use core::fmt::Write;
            let _ = write!($crate::util::LogWriter, "{}:{}:warn: ", file!(), line!());
            let _ = writeln!($crate::util::LogWriter, $($arg)*);
        }
    };
}

#[macro_export]
macro_rules! LFS_ERROR {
    ($($arg:tt)*) => {
        #[cfg(not(feature = "lfs_no_error"))]
        {
            use core::fmt::Write;
            let _ = write!($crate::util::LogWriter, "{}:{}:error: ", file!(), line!());
            let _ = writeln!($crate::util::LogWriter, $($arg)*);
        }
    };
}

#[macro_export]
macro_rules! LFS_ASSERT {
    ($test:expr) => {
        #[cfg(not(feature = "lfs_no_assert"))]
        {
            if !$test {
                panic!("Assertion failed: {}", stringify!($test));
            }
        }
    };
}

#[inline]
pub fn lfs_max(a: u32, b: u32) -> u32 {
    a.max(b)
}

#[inline]
pub fn lfs_min(a: u32, b: u32) -> u32 {
    a.min(b)
}

#[inline]
pub fn lfs_aligndown(a: u32, alignment: u32) -> u32 {
    a - (a % alignment)
}

#[inline]
pub fn lfs_alignup(a: u32, alignment: u32) -> u32 {
    lfs_aligndown(a + alignment - 1, alignment)
}

#[inline]
pub fn lfs_npw2(mut a: u32) -> u32 {
    if a == 0 {
        return 0;
    }
    a -= 1;
    let mut r = 0;
    let mut s;
    s = (a > 0xffff) as u32 * 4; a >>= s; r |= s;
    s = (a > 0xff) as u32 * 3; a >>= s; r |= s;
    s = (a > 0xf) as u32 * 2; a >>= s; r |= s;
    s = (a > 0x3) as u32 * 1; a >>= s; r |= s;
    (r | (a >> 1)) + 1
}

#[inline]
pub fn lfs_ctz(a: u32) -> u32 {
    a.trailing_zeros()
}

#[inline]
pub fn lfs_popc(a: u32) -> u32 {
    a.count_ones()
}

#[inline]
pub fn lfs_scmp(a: u32, b: u32) -> i32 {
    a.wrapping_sub(b) as i32
}

#[inline]
pub fn lfs_fromle32(a: u32) -> u32 {
    u32::from_le_bytes(a.to_le_bytes())
}

#[inline]
pub fn lfs_tole32(a: u32) -> u32 {
    a.to_le()
}

#[inline]
pub fn lfs_frombe32(a: u32) -> u32 {
    u32::from_be_bytes(a.to_be_bytes())
}

#[inline]
pub fn lfs_tobe32(a: u32) -> u32 {
    a.to_be()
}

#[cfg(not(feature = "lfs_no_malloc"))]
pub fn lfs_malloc(size: usize) -> *mut core::ffi::c_void {
    unsafe {
        let layout = Layout::from_size_align(size, 1).unwrap();
        alloc(layout) as *mut _
    }
}

#[cfg(not(feature = "lfs_no_malloc"))]
pub fn lfs_free(p: *mut core::ffi::c_void) {
    unsafe {
        dealloc(p as *mut u8, Layout::from_size_align(0, 1).unwrap());
    }
}

// 日志输出抽象（需用户实现）
#[cfg(any(
    feature = "lfs_trace",
    not(feature = "lfs_no_debug"),
    not(feature = "lfs_no_warn"),
    not(feature = "lfs_no_error")
))]
pub struct LogWriter;

#[cfg(any(
    feature = "lfs_trace",
    not(feature = "lfs_no_debug"),
    not(feature = "lfs_no_warn"),
    not(feature = "lfs_no_error")
))]
impl core::fmt::Write for LogWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // Attention: This should be implemented by user. [println!()] is not [no_std] compatible.
        println!("{}", s);
        Ok(())
    }
}

const RTABLE: [u32; 16] = [
    0x00000000, 0x1db71064, 0x3b6e20c8, 0x26d930ac,
    0x76dc4190, 0x6b6b51f4, 0x4db26158, 0x5005713c,
    0xedb88320, 0xf00f9344, 0xd6d6a3e8, 0xcb61b38c,
    0x9b64c2b0, 0x86d3d2d4, 0xa00ae278, 0xbdbdf21c,
];

pub fn lfs_crc(mut crc: u32, buffer: &[u8]) -> u32 {
    for &byte in buffer {
        crc = (crc >> 4) ^ RTABLE[((crc ^ u32::from(byte)) & 0x0f) as usize];
        crc = (crc >> 4) ^ RTABLE[((crc ^ (u32::from(byte) >> 4)) & 0x0f) as usize];
    }
    crc
}