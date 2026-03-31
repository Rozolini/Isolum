use std::fs;
use vmm_core::elf::load_elf;
use vmm_core::event_loop::EventLoop;
use vmm_core::memory::GuestMemory;
use vmm_core::partition::Partition;
use vmm_core::uart::Uart;
use vmm_core::vcpu::Vcpu;
use whpx_bindings::api::is_hypervisor_present;
use windows::Win32::System::Hypervisor::{WHvX64RegisterRsp, WHV_REGISTER_VALUE};

/// End-to-End test verifying the complete lifecycle of the hypervisor.
/// It loads a compiled ELF payload, initializes the VM, executes the payload,
/// and verifies the serial port (UART) output.
#[test]
fn test_e2e_full_system_execution() {
    // Skip if the host does not support hardware virtualization.
    let present = is_hypervisor_present().expect("Failed to query hypervisor state");
    if !present {
        println!("WHPX not present. Skipping E2E test.");
        return;
    }

    // 1. Initialize core hypervisor components.
    let partition = Partition::new().expect("Failed to create WHPX partition");
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 2 * 1024 * 1024)
        .expect("Failed to allocate 2MB guest memory");

    // 2. Setup memory translation (Paging).
    let pml4_addr = guest_mem
        .setup_identity_paging()
        .expect("Failed to setup identity paging");

    // 3. Load the pre-compiled guest payload (ELF format).
    // The payload must compute 0x40 + 0x0B and output 'K' (0x4B) to COM1.
    let payload_path = "target/x86_64-unknown-none/debug/payload";
    let elf_bytes = fs::read(payload_path)
        .expect("Failed to read ELF payload. Ensure 'cargo build -p payload' was run.");

    let entry_point = load_elf(&elf_bytes, &mut guest_mem).expect("Failed to load ELF segments");

    // 4. Initialize the Virtual CPU.
    let vcpu = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU");
    vcpu.init_long_mode(entry_point, pml4_addr)
        .expect("Failed to initialize x86_64 Long Mode");

    // Configure the stack pointer (RSP) to the top of the allocated 2MB memory.
    let mut rsp_val = WHV_REGISTER_VALUE::default();
    rsp_val.Reg64 = 2 * 1024 * 1024;
    vcpu.set_registers(&[WHvX64RegisterRsp], &[rsp_val])
        .expect("Failed to set RSP register");

    // 5. Attach peripherals and execute the VM loop.
    let mut uart = Uart::new();
    let event_loop = EventLoop::new(&vcpu);

    event_loop
        .run_with_uart(Some(&mut uart))
        .expect("Event loop terminated with an unexpected error");

    // 6. Verify hardware state changes and peripheral output.
    assert_eq!(
        uart.get_buffer(),
        b"K",
        "E2E Test Failed: UART output did not match expected payload computation"
    );
}
