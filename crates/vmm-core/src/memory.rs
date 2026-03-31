use std::ffi::c_void;
use std::ptr::null_mut;
use whpx_bindings::api::{map_gpa_range, unmap_gpa_range};
use windows::Win32::System::Hypervisor::{WHV_MAP_GPA_RANGE_FLAGS, WHV_PARTITION_HANDLE};
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};

/// Manages guest physical memory allocation and WHPX mapping with RAII semantics.
pub struct GuestMemory {
    partition_handle: WHV_PARTITION_HANDLE,
    hva: *mut c_void,
    gpa: u64,
    size: u64,
}

impl GuestMemory {
    /// Allocates host memory and maps it into the guest physical address space.
    pub fn new(
        partition_handle: WHV_PARTITION_HANDLE,
        gpa: u64,
        size: u64,
    ) -> Result<Self, windows::core::Error> {
        let hva = unsafe {
            VirtualAlloc(
                Some(null_mut()),
                size as usize,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };

        if hva.is_null() {
            return Err(windows::core::Error::from_thread());
        }

        // WHvMapGpaRangeFlagRead (1) | WHvMapGpaRangeFlagWrite (2) | WHvMapGpaRangeFlagExecute (4) = 7
        let flags = WHV_MAP_GPA_RANGE_FLAGS(7);

        let map_res = unsafe { map_gpa_range(partition_handle, hva, gpa, size, flags) };
        if let Err(e) = map_res {
            unsafe {
                let _ = VirtualFree(hva, 0, MEM_RELEASE);
            }
            return Err(e);
        }

        Ok(Self {
            partition_handle,
            hva,
            gpa,
            size,
        })
    }

    /// Reads data from guest memory into the provided buffer.
    pub fn read_bytes(&self, offset: u64, buf: &mut [u8]) -> Result<(), &'static str> {
        if offset + buf.len() as u64 > self.size {
            return Err("Read exceeds allocated memory bounds");
        }

        unsafe {
            let src = (self.hva as *const u8).add(offset as usize);
            std::ptr::copy_nonoverlapping(src, buf.as_mut_ptr(), buf.len());
        }

        Ok(())
    }

    /// Writes data from the provided buffer into guest memory.
    pub fn write_bytes(&mut self, offset: u64, data: &[u8]) -> Result<(), &'static str> {
        if offset + data.len() as u64 > self.size {
            return Err("Payload exceeds allocated memory bounds");
        }

        unsafe {
            let dst = (self.hva as *mut u8).add(offset as usize);
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
        }

        Ok(())
    }

    /// Configures 4-level identity paging for the first 2MB.
    /// Returns the physical address of the PML4 table.
    pub fn setup_identity_paging(&mut self) -> Result<u64, String> {
        let pml4_addr: u64 = 0x10000;
        let pdpt_addr: u64 = 0x11000;
        let pd_addr: u64 = 0x12000;
        let pt_addr: u64 = 0x13000;

        if self.size < pt_addr + 0x1000 {
            return Err("Guest memory too small for paging tables".into());
        }

        let write_u64 = |mem: &mut Self, offset: u64, val: u64| {
            let bytes = val.to_le_bytes();
            let _ = mem.write_bytes(offset, &bytes);
        };

        // PML4[0] -> PDPT (Present | R/W = 0x3)
        write_u64(self, pml4_addr, pdpt_addr | 0x3);
        // PDPT[0] -> PD (Present | R/W = 0x3)
        write_u64(self, pdpt_addr, pd_addr | 0x3);
        // PD[0] -> PT (Present | R/W = 0x3)
        write_u64(self, pd_addr, pt_addr | 0x3);

        // PT[0..512] -> Map first 2MB pages (Present | R/W = 0x3)
        for i in 0u64..512 {
            let physical_page = i * 0x1000;
            write_u64(self, pt_addr + (i * 8), physical_page | 0x3);
        }

        Ok(pml4_addr)
    }

    /// Returns a mutable pointer to the host virtual address.
    #[inline]
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.hva as *mut u8
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        // Unmap the GPA range and release the underlying host memory.
        let _ = unmap_gpa_range(self.partition_handle, self.gpa, self.size);
        unsafe {
            let _ = VirtualFree(self.hva, 0, MEM_RELEASE);
        }
    }
}
