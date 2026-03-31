use std::ffi::c_void;
use std::mem::size_of;
use windows::Win32::System::Hypervisor::{
    WHvCapabilityCodeHypervisorPresent, WHvCreatePartition, WHvCreateVirtualProcessor,
    WHvDeletePartition, WHvGetCapability, WHvGetVirtualProcessorRegisters, WHvMapGpaRange,
    WHvRunVirtualProcessor, WHvSetPartitionProperty, WHvSetVirtualProcessorRegisters,
    WHvSetupPartition, WHvUnmapGpaRange, WHV_CAPABILITY_FEATURES, WHV_MAP_GPA_RANGE_FLAGS,
    WHV_PARTITION_HANDLE, WHV_PARTITION_PROPERTY, WHV_PARTITION_PROPERTY_CODE, WHV_REGISTER_NAME,
    WHV_REGISTER_VALUE,
};

/// Queries the host to verify if the Windows Hypervisor Platform (WHPX) is available and enabled.
pub fn is_hypervisor_present() -> Result<bool, windows::core::Error> {
    let mut features = WHV_CAPABILITY_FEATURES::default();
    let mut return_size: u32 = 0;

    unsafe {
        WHvGetCapability(
            WHvCapabilityCodeHypervisorPresent,
            &mut features as *mut _ as *mut c_void,
            size_of::<WHV_CAPABILITY_FEATURES>() as u32,
            Some(&mut return_size as *mut u32),
        )?;
    }

    // Direct bitwise check for HypervisorPresent (Bit 0).
    Ok(unsafe { features.AsUINT64 & 1 } != 0)
}

/// Instantiates a new, uninitialized WHPX partition.
pub fn create_partition() -> Result<WHV_PARTITION_HANDLE, windows::core::Error> {
    unsafe { WHvCreatePartition() }
}

/// Configures a specific WHPX partition property.
/// Must precede `setup_partition` for immutable properties.
pub fn set_partition_property(
    handle: WHV_PARTITION_HANDLE,
    property_code: WHV_PARTITION_PROPERTY_CODE,
    property: &WHV_PARTITION_PROPERTY,
) -> Result<(), windows::core::Error> {
    unsafe {
        WHvSetPartitionProperty(
            handle,
            property_code,
            property as *const _ as *const c_void,
            size_of::<WHV_PARTITION_PROPERTY>() as u32,
        )
    }
}

/// Finalizes partition initialization, enabling memory mapping and vCPU creation.
pub fn setup_partition(handle: WHV_PARTITION_HANDLE) -> Result<(), windows::core::Error> {
    unsafe { WHvSetupPartition(handle) }
}

/// Destroys the partition and releases associated host resources.
pub fn delete_partition(handle: WHV_PARTITION_HANDLE) -> Result<(), windows::core::Error> {
    unsafe { WHvDeletePartition(handle) }
}

/// Maps a host virtual address (HVA) range into the guest physical address (GPA) space.
///
/// # Safety
/// `v_address` must point to a valid, page-aligned host allocation that outlives the mapping.
pub unsafe fn map_gpa_range(
    handle: WHV_PARTITION_HANDLE,
    v_address: *const c_void,
    gpa: u64,
    size: u64,
    flags: WHV_MAP_GPA_RANGE_FLAGS,
) -> Result<(), windows::core::Error> {
    WHvMapGpaRange(handle, v_address, gpa, size, flags)
}

/// Removes a GPA range mapping from the partition.
pub fn unmap_gpa_range(
    handle: WHV_PARTITION_HANDLE,
    gpa: u64,
    size: u64,
) -> Result<(), windows::core::Error> {
    unsafe { WHvUnmapGpaRange(handle, gpa, size) }
}

/// Instantiates a virtual processor (vCPU) within the target partition.
pub fn create_virtual_processor(
    handle: WHV_PARTITION_HANDLE,
    vp_index: u32,
) -> Result<(), windows::core::Error> {
    unsafe { WHvCreateVirtualProcessor(handle, vp_index, 0) }
}

/// Executes the vCPU until a VMExit occurs.
///
/// # Safety
/// `exit_context` must be a valid pointer to a `WHV_RUN_VP_EXIT_CONTEXT` struct.
pub unsafe fn run_virtual_processor(
    handle: WHV_PARTITION_HANDLE,
    vp_index: u32,
    exit_context: *mut c_void,
    exit_context_size: u32,
) -> Result<(), windows::core::Error> {
    WHvRunVirtualProcessor(handle, vp_index, exit_context, exit_context_size)
}

/// Writes vCPU register states.
///
/// # Safety
/// Pointer arrays must be valid and contain at least `register_count` elements.
pub unsafe fn set_virtual_processor_registers(
    handle: WHV_PARTITION_HANDLE,
    vp_index: u32,
    register_names: *const WHV_REGISTER_NAME,
    register_count: u32,
    register_values: *const WHV_REGISTER_VALUE,
) -> Result<(), windows::core::Error> {
    WHvSetVirtualProcessorRegisters(
        handle,
        vp_index,
        register_names,
        register_count,
        register_values,
    )
}

/// Reads vCPU register states.
///
/// # Safety
/// Pointer arrays must be valid and contain at least `register_count` elements.
pub unsafe fn get_virtual_processor_registers(
    handle: WHV_PARTITION_HANDLE,
    vp_index: u32,
    register_names: *const WHV_REGISTER_NAME,
    register_count: u32,
    register_values: *mut WHV_REGISTER_VALUE,
) -> Result<(), windows::core::Error> {
    WHvGetVirtualProcessorRegisters(
        handle,
        vp_index,
        register_names,
        register_count,
        register_values,
    )
}
