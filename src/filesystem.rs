use crate::block_device::{BlockDevice, BLOCK_SIZE, NUM_BLOCKS};
use std::io::{Error, ErrorKind, Result};

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; 24],
    pub start_block: u16,
    pub size: u32,
    pub is_dir: bool,
    _padding: u8,
}

impl Default for DirEntry {
    fn default() -> Self {
        DirEntry {
            name: [0; 24],
            start_block: 0,
            size: 0,
            is_dir: false,
            _padding: 0,
        }
    }
}

#[repr(C)]
pub struct SuperBlock {
    bitmap: [u8; NUM_BLOCKS / 8],
    root_dir: u16,
    _padding: [u8; BLOCK_SIZE - (NUM_BLOCKS / 8) - 2],
}

pub struct FileSystem {
    device: BlockDevice,
    super_block: SuperBlock,
    current_dir: u16,
}

impl FileSystem {
    pub fn mount(device: BlockDevice) -> Result<Self> {
        let mut super_block = SuperBlock {
            bitmap: [0; NUM_BLOCKS / 8],
            root_dir: 1,
            _padding: [0; BLOCK_SIZE - (NUM_BLOCKS / 8) - 2],
        };

        if device.read_block(0, unsafe {
            std::slice::from_raw_parts_mut(
                &mut super_block as *mut _ as *mut u8,
                BLOCK_SIZE,
            )
        }).is_err() {
            super_block.bitmap[0] = 0b0000_0011;
            device.write_block(0, unsafe {
                std::slice::from_raw_parts(
                    &super_block as *const _ as *const u8,
                    BLOCK_SIZE,
                )
            })?;

            let empty_dir = [DirEntry::default(); BLOCK_SIZE / std::mem::size_of::<DirEntry>()];
            device.write_block(1, unsafe {
                std::slice::from_raw_parts(
                    empty_dir.as_ptr() as *const u8,
                    BLOCK_SIZE,
                )
            })?;
        }

        Ok(Self {
            device,
            super_block,
            current_dir: 1,
        })
    }

    fn allocate_block(&mut self) -> Result<u16> {
        for (byte_idx, byte) in self.super_block.bitmap.iter_mut().enumerate() {
            for bit in 0..8 {
                if (*byte & (1 << bit)) == 0 {
                    *byte |= 1 << bit;
                    return Ok((byte_idx * 8 + bit) as u16);
                }
            }
        }
        Err(Error::new(ErrorKind::Other, "No free blocks"))
    }

    pub fn create_dir(&mut self, name: &str) -> Result<()> {
        let new_block = self.allocate_block()?;
        let empty_dir = [DirEntry::default(); BLOCK_SIZE / std::mem::size_of::<DirEntry>()];
        self.device.write_block(new_block as usize, unsafe {
            std::slice::from_raw_parts(
                empty_dir.as_ptr() as *const u8,
                BLOCK_SIZE,
            )
        })?;

        let mut buf = [0u8; BLOCK_SIZE];
        self.device.read_block(self.current_dir as usize, &mut buf)?;
        let dir_entries = unsafe {
            std::slice::from_raw_parts_mut(
                buf.as_mut_ptr() as *mut DirEntry,
                BLOCK_SIZE / std::mem::size_of::<DirEntry>(),
            )
        };

        for entry in dir_entries.iter_mut() {
            if entry.start_block == 0 {
                let name_bytes = &mut entry.name;
                let name_str = name.as_bytes();
                let len = name_str.len().min(23);
                name_bytes[..len].copy_from_slice(&name_str[..len]);
                entry.start_block = new_block;
                entry.is_dir = true;
                break;
            }
        }

        self.device.write_block(self.current_dir as usize, unsafe {
            std::slice::from_raw_parts(
                dir_entries.as_ptr() as *const u8,
                BLOCK_SIZE,
            )
        })
    }

    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let block = self.allocate_block()?;
        let trimmed_data = &data[..data.len().min(BLOCK_SIZE)];
        
        self.device.write_block(block as usize, trimmed_data)?;

        let mut buf = [0u8; BLOCK_SIZE];
        self.device.read_block(self.current_dir as usize, &mut buf)?;
        let dir_entries = unsafe {
            std::slice::from_raw_parts_mut(
                buf.as_mut_ptr() as *mut DirEntry,
                BLOCK_SIZE / std::mem::size_of::<DirEntry>(),
            )
        };

        for entry in dir_entries.iter_mut() {
            if entry.start_block == 0 {
                let name_bytes = &mut entry.name;
                let name_str = path.as_bytes();
                let len = name_str.len().min(23);
                name_bytes[..len].copy_from_slice(&name_str[..len]);
                entry.start_block = block;
                entry.size = trimmed_data.len() as u32;
                break;
            }
        }

        self.device.write_block(self.current_dir as usize, unsafe {
            std::slice::from_raw_parts(
                dir_entries.as_ptr() as *const u8,
                BLOCK_SIZE,
            )
        })
    }

    pub fn list_dir(&self) -> Result<Vec<String>> {
        let mut buf = [0u8; BLOCK_SIZE];
        self.device.read_block(self.current_dir as usize, &mut buf)?;

        let dir_entries = unsafe {
            std::slice::from_raw_parts(
                buf.as_ptr() as *const DirEntry,
                BLOCK_SIZE / std::mem::size_of::<DirEntry>(),
            )
        };

        Ok(dir_entries
            .iter()
            .filter(|e| e.start_block != 0)
            .map(|e| {
                String::from_utf8_lossy(&e.name)
                    .trim_end_matches('\0')
                    .to_string()
            })
            .collect())
    }
}