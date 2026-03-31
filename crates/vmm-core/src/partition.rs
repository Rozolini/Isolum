use whpx_bindings::api::{
    create_partition, delete_partition, set_partition_property, setup_partition,
};
use windows::Win32::System::Hypervisor::{
    WHvPartitionPropertyCodeExceptionExitBitmap, WHvPartitionPropertyCodeExtendedVmExits,
    WHvPartitionPropertyCodeProcessorCount, WHV_PARTITION_HANDLE, WHV_PARTITION_PROPERTY,
};

/// WHPX partition wrapper providing RAII semantics and handle management.
pub struct Partition {
    handle: WHV_PARTITION_HANDLE,
}

impl Partition {
    /// Creates a new partition configured for a single virtual processor.
    pub fn new() -> Result<Self, windows::core::Error> {
        Self::create(1)
    }

    /// Allocates and configures the underlying WHPX partition.
    fn create(processor_count: u32) -> Result<Self, windows::core::Error> {
        let handle = create_partition()?;

        // Configure the number of virtual processors.
        let mut property = WHV_PARTITION_PROPERTY::default();
        property.ProcessorCount = processor_count;
        if let Err(e) =
            set_partition_property(handle, WHvPartitionPropertyCodeProcessorCount, &property)
        {
            let _ = delete_partition(handle);
            return Err(e);
        }

        // Enable extended VM exits (e.g., CPUID interception).
        let mut ext_prop = WHV_PARTITION_PROPERTY::default();
        ext_prop.ExceptionExitBitmap = (1 << 0) | (1 << 2);
        if let Err(e) =
            set_partition_property(handle, WHvPartitionPropertyCodeExtendedVmExits, &ext_prop)
        {
            let _ = delete_partition(handle);
            return Err(e);
        }

        // Intercept critical hardware exceptions: #DF (8), #GP (13), #PF (14).
        let mut exc_prop = WHV_PARTITION_PROPERTY::default();
        exc_prop.ExceptionExitBitmap = (1 << 14) | (1 << 8) | (1 << 13);
        if let Err(e) = set_partition_property(
            handle,
            WHvPartitionPropertyCodeExceptionExitBitmap,
            &exc_prop,
        ) {
            let _ = delete_partition(handle);
            return Err(e);
        }

        // Hardware APIC emulation skipped to support broader host compatibility.

        // Finalize partition initialization.
        if let Err(e) = setup_partition(handle) {
            let _ = delete_partition(handle);
            return Err(e);
        }

        Ok(Self { handle })
    }

    /// Returns the raw WHV_PARTITION_HANDLE.
    #[inline]
    pub fn as_raw(&self) -> WHV_PARTITION_HANDLE {
        self.handle
    }
}

impl Drop for Partition {
    fn drop(&mut self) {
        // Ensure the partition is properly destroyed by the hypervisor.
        let _ = delete_partition(self.handle);
    }
}

// WHV_PARTITION_HANDLE is thread-safe at the OS level.
unsafe impl Send for Partition {}
unsafe impl Sync for Partition {}
