use crate::memory::GuestMemory;
use std::fs;
use std::path::Path;

/// Loads a raw binary payload into guest physical memory at the specified GPA offset.
pub fn load_flat_binary<P: AsRef<Path>>(
    memory: &mut GuestMemory,
    offset: u64,
    path: P,
) -> Result<(), String> {
    // Read the entire binary into a local host buffer.
    let data = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;

    // Copy the buffer into the guest physical address space.
    memory.write_bytes(offset, &data).map_err(|e| e.to_string())
}
