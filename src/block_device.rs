use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

pub const BLOCK_SIZE: usize = 512;
pub const NUM_BLOCKS: usize = 1024;

pub struct BlockDevice {
    file: Mutex<File>,
}

impl BlockDevice {
    pub fn new(img_path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(img_path)?;
        file.set_len((BLOCK_SIZE * NUM_BLOCKS) as u64)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn read_block(&self, block_id: usize, buf: &mut [u8]) -> std::io::Result<()> {
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SIZE) as u64))?;
        file.read_exact(buf)
    }

    pub fn write_block(&self, block_id: usize, buf: &[u8]) -> std::io::Result<()> {
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SIZE) as u64))?;
        file.write_all(buf)
    }

    pub fn quick_clear(&self) -> std::io::Result<()> {
        let file = self.file.lock().unwrap();
        file.set_len(0)?;
        file.set_len((BLOCK_SIZE * NUM_BLOCKS) as u64)?;
        Ok(())
    }
}