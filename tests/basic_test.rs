use littlefs_rust::{block_device::BlockDevice, lfs_impl_ds::FileSystem};
use std::fs::remove_file;

#[test]
fn test_basic_operations() -> std::io::Result<()> {
    let path = "test.img";
    let device = BlockDevice::new(&path)?;
    device.quick_clear()?;
    let mut fs = FileSystem::mount(device)?;

    // Test directory creation
    fs.create_dir("docs")?;
    // Test file writing
    fs.write_file("docs/note.txt", b"Hello World")?;

    assert!(fs.list_dir()?.contains(&"docs/note.txt".into()));

    remove_file(path)?;
    Ok(())
}