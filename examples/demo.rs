use littlefs_rust::{block_device::BlockDevice, lfs_impl_ds::FileSystem};
use std::fs::remove_file;

fn main() -> std::io::Result<()> {
    let path = "disk.img";
    let device = BlockDevice::new(&path)?;
    device.quick_clear()?;
    let mut fs = FileSystem::mount(device)?;

    fs.create_dir("documents")?;
    println!("Root directory contents: {:?}", fs.list_dir()?);
    fs.write_file("documents/note.txt", b"Sample text")?;

    println!("Root directory contents: {:?}", fs.list_dir()?);
    remove_file(path)?;
    Ok(())
}