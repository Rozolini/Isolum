use std::env;
use std::fs;
use std::process;

use vmm_core::elf::load_elf;
use vmm_core::event_loop::EventLoop;
use vmm_core::memory::GuestMemory;
use vmm_core::partition::Partition;
use vmm_core::uart::Uart;
use vmm_core::vcpu::Vcpu;
use whpx_bindings::api::is_hypervisor_present;
use windows::Win32::System::Hypervisor::{WHvX64RegisterRsp, WHV_REGISTER_VALUE};

// Host application entry point for executing guest payloads.
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path-to-elf-payload>", args[0]);
        process::exit(1);
    }

    let payload_path = &args[1];

    // Verify hardware virtualization support before proceeding.
    if !is_hypervisor_present().unwrap_or(false) {
        eprintln!("Error: WHPX is not enabled on this host.");
        process::exit(1);
    }

    let elf_bytes = fs::read(payload_path).unwrap_or_else(|err| {
        eprintln!("Error reading payload file: {}", err);
        process::exit(1);
    });

    // Initialize the WHPX partition and allocate 2MB of guest physical memory.
    let partition = Partition::new().expect("Failed to create WHPX partition");
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 2 * 1024 * 1024)
        .expect("Failed to allocate guest memory");

    // Establish 4-level identity paging and map ELF segments into GPA space.
    let pml4_addr = guest_mem
        .setup_identity_paging()
        .expect("Failed to setup paging");
    let entry_point = load_elf(&elf_bytes, &mut guest_mem).expect("Failed to load ELF");

    // Initialize vCPU execution state for x86_64 Long Mode.
    let vcpu = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU");
    vcpu.init_long_mode(entry_point, pml4_addr)
        .expect("Failed to init long mode");

    // Initialize the guest stack pointer (RSP) to the top of allocated memory.
    let mut rsp_val = WHV_REGISTER_VALUE::default();
    rsp_val.Reg64 = 2 * 1024 * 1024;
    vcpu.set_registers(&[WHvX64RegisterRsp], &[rsp_val])
        .expect("Failed to set RSP");

    // Attach telemetry devices and transfer control to the guest.
    let mut uart = Uart::new();
    let event_loop = EventLoop::new(&vcpu);

    println!("Starting VM execution...");
    event_loop
        .run_with_uart(Some(&mut uart))
        .expect("Event loop failed");

    println!("\nVM execution halted.");
    println!(
        "UART Output:\n{}",
        String::from_utf8_lossy(uart.get_buffer())
    );
}
