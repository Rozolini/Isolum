use crate::memory::GuestMemory;
use goblin::elf::{program_header::PT_LOAD, Elf};

/// Parses an ELF64 binary and loads its PT_LOAD segments into guest memory.
/// Returns the entry point address (e_entry).
pub fn load_elf(elf_bytes: &[u8], guest_memory: &mut GuestMemory) -> Result<u64, &'static str> {
    let elf = Elf::parse(elf_bytes).map_err(|_| "Failed to parse ELF file")?;

    if !elf.is_64 {
        return Err("Only 64-bit ELF binaries are supported");
    }

    for ph in elf.program_headers {
        if ph.p_type == PT_LOAD {
            let offset = ph.p_offset as usize;
            let filesz = ph.p_filesz as usize;
            let memsz = ph.p_memsz as usize;
            let paddr = ph.p_paddr; // Keep as u64

            if offset + filesz > elf_bytes.len() {
                return Err("ELF segment bounds exceed file size");
            }

            // Write the segment data to guest physical memory.
            let segment_data = &elf_bytes[offset..offset + filesz];
            guest_memory.write_bytes(paddr, segment_data)?;

            // Zero-initialize the remaining memory (e.g., .bss section).
            if memsz > filesz {
                let zero_pad = vec![0u8; memsz - filesz];
                guest_memory.write_bytes(paddr + filesz as u64, &zero_pad)?;
            }
        }
    }

    Ok(elf.entry)
}
