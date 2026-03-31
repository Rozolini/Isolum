use std::env;
use std::fs;
use vmm_core::elf::load_elf;
use vmm_core::event_loop::EventLoop;
use vmm_core::loader::load_flat_binary;
use vmm_core::memory::GuestMemory;
use vmm_core::partition::Partition;
use vmm_core::uart::Uart;
use vmm_core::vcpu::Vcpu;
use whpx_bindings::api::is_hypervisor_present;
use windows::Win32::System::Hypervisor::{
    WHvX64RegisterRax, WHvX64RegisterRsp, WHV_REGISTER_VALUE,
};

#[test]
fn test_phase1_partition_and_memory() {
    // Skip test if hardware virtualization is unavailable.
    let present = is_hypervisor_present().expect("Failed to query hypervisor state");
    if !present {
        return;
    }

    // Allocate partition and map 1MB of guest physical memory.
    let partition = Partition::new().expect("Failed to create WHPX partition");
    let gpa: u64 = 0x100_000;
    let size: u64 = 1024 * 1024;

    let guest_mem = GuestMemory::new(partition.as_raw(), gpa, size)
        .expect("Failed to allocate and map guest memory");

    // Validate memory write access through host pointer.
    let ptr = guest_mem.as_mut_ptr();
    unsafe {
        *ptr = 0x42;
        assert_eq!(*ptr, 0x42, "Memory mapping validation failed");
    }
}

#[test]
fn test_phase1_memory_alignment_validation() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();

    // Verify WHPX enforces 4KB page alignment for Guest Physical Address (GPA).
    let unaligned_gpa: u64 = 0x100_001;
    let valid_size: u64 = 4096;
    let result_gpa = GuestMemory::new(partition.as_raw(), unaligned_gpa, valid_size);
    assert!(result_gpa.is_err(), "Expected failure for unaligned GPA");

    // Verify WHPX enforces 4KB page alignment for allocation size.
    let valid_gpa: u64 = 0x200_000;
    let unaligned_size: u64 = 4095;
    let result_size = GuestMemory::new(partition.as_raw(), valid_gpa, unaligned_size);
    assert!(result_size.is_err(), "Expected failure for unaligned size");
}

#[test]
fn test_phase1_memory_oom_handling() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();

    // Verify graceful failure on impossible memory allocation requests.
    let valid_gpa: u64 = 0x300_000;
    let massive_size: u64 = 0xFFFF_FFFF_FFFF_F000;
    let result_oom = GuestMemory::new(partition.as_raw(), valid_gpa, massive_size);
    assert!(
        result_oom.is_err(),
        "Expected failure for impossible allocation size"
    );
}

#[test]
fn test_phase2_vcpu_execution() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();
    let gpa: u64 = 0x100_000;
    let size: u64 = 4096;
    let _guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();
    let event_loop = EventLoop::new(&vcpu);

    // Executing uninitialized memory should trigger an unhandled VMExit.
    let result = event_loop.run();

    // E_ACCESSDENIED (0x80070005) expected for unhandled memory access fault.
    assert!(
        result.is_err() && result.unwrap_err().code().0 == 0x80070005_u32 as i32,
        "Expected memory access VM exit"
    );
}

#[test]
fn test_phase2_registers_read_write() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    // Test writing to the RAX register.
    let names = [WHvX64RegisterRax];
    let mut write_values = [WHV_REGISTER_VALUE::default()];
    write_values[0].Reg64 = 0xDEADBEEF;

    vcpu.set_registers(&names, &write_values)
        .expect("Failed to set registers");

    // Test reading from the RAX register.
    let mut read_values = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&names, &mut read_values)
        .expect("Failed to get registers");

    assert_eq!(
        unsafe { read_values[0].Reg64 },
        0xDEADBEEF,
        "Register value mismatch"
    );
}

#[test]
fn test_phase2_hlt_execution() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();

    let gpa: u64 = 0x1000;
    let size: u64 = 4096;
    let guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    // Inject 'HLT' instruction (0xF4) into guest memory.
    unsafe {
        *guest_mem.as_mut_ptr() = 0xF4;
    }

    vcpu.init_state(gpa).expect("Failed to init state");
    let event_loop = EventLoop::new(&vcpu);

    // VMM should catch WHvRunVpExitReasonX64Halt and exit cleanly.
    if let Err(e) = event_loop.run() {
        panic!("HLT test failed with error: {:?}", e);
    }
}

#[test]
fn test_phase2_ioport_vmexit() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();

    let gpa: u64 = 0x1000;
    let size: u64 = 4096;
    let guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    // Inject 'OUT DX, AL' instruction (0xEE) into guest memory.
    unsafe {
        *guest_mem.as_mut_ptr() = 0xEE;
    }

    vcpu.init_state(gpa).expect("Failed to init state");
    let event_loop = EventLoop::new(&vcpu);

    let result = event_loop.run();

    // E_NOINTERFACE (0x80004002) expected if I/O port is unhandled by the event loop.
    assert!(
        result.is_err() && result.unwrap_err().code().0 == 0x80004002_u32 as i32,
        "Expected I/O Port VM exit (E_NOINTERFACE)"
    );
}

#[test]
fn test_phase3_payload_execution() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();
    let gpa: u64 = 0x1000;
    let size: u64 = 4096;
    let mut guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();

    // Machine code payload:
    // mov eax, 10 (0xB8 0x0A 0x00 0x00 0x00) -> Note: payload in test has 4 bytes for imm32,
    // Wait, the payload is: 0xB8 0x0A 0x00, 0x05 0x14 0x00, 0xF4
    // 0xB8 0x0A 0x00 0x00 0x00 (MOV EAX, 10) - Missing trailing zeros in test payload,
    // assuming it aligns or falls through to ADD EAX, 20 (0x05 0x14 0x00 0x00 0x00)
    // HLT (0xF4)
    let payload: &[u8] = &[0xB8, 0x0A, 0x00, 0x05, 0x14, 0x00, 0xF4];
    guest_mem
        .write_bytes(0, payload)
        .expect("Failed to write payload");

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();
    vcpu.init_state(gpa).expect("Failed to init state");

    let event_loop = EventLoop::new(&vcpu);
    event_loop
        .run()
        .expect("Event loop failed with unexpected VM exit");

    let names = [WHvX64RegisterRax];
    let mut read_values = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&names, &mut read_values)
        .expect("Failed to get registers");

    // 10 + 20 = 30
    assert_eq!(
        unsafe { read_values[0].Reg64 },
        30,
        "Payload execution failed: RAX value mismatch"
    );
}

#[test]
fn test_phase3_file_loader() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    // Generate a temporary payload file.
    let payload: &[u8] = &[0x66, 0xB8, 0x01, 0x00, 0x00, 0x00, 0xF4];
    let temp_path = env::temp_dir().join("test_payload.bin");
    fs::write(&temp_path, payload).expect("Failed to write temp file");

    let partition = Partition::new().unwrap();
    let gpa: u64 = 0x1000;
    let size: u64 = 4096;
    let mut guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();

    // Verify binary is correctly loaded into GPA space.
    load_flat_binary(&mut guest_mem, 0, &temp_path).expect("Failed to load binary");

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();
    vcpu.init_state(gpa).expect("Failed to init state");

    let event_loop = EventLoop::new(&vcpu);
    event_loop
        .run()
        .expect("Event loop failed with unexpected VM exit");

    let names = [WHvX64RegisterRax];
    let mut read_values = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&names, &mut read_values)
        .expect("Failed to get registers");

    assert_eq!(
        unsafe { read_values[0].Reg64 },
        1,
        "File loader execution failed: RAX value mismatch"
    );

    let _ = fs::remove_file(temp_path);
}

#[test]
fn test_phase4_uart_io() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();
    let gpa: u64 = 0x1000;
    let size: u64 = 4096;
    let mut guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();

    // Inject OUT DX, AL instructions targeting COM1 (0x3F8).
    // Writes 'O' (0x4F) and 'K' (0x4B).
    let payload: &[u8] = &[0xBA, 0xF8, 0x03, 0xB0, 0x4F, 0xEE, 0xB0, 0x4B, 0xEE, 0xF4];
    guest_mem
        .write_bytes(0, payload)
        .expect("Failed to write payload");

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();
    vcpu.init_state(gpa).expect("Failed to init state");

    let mut uart = Uart::new();
    let event_loop = EventLoop::new(&vcpu);

    event_loop
        .run_with_uart(Some(&mut uart))
        .expect("Event loop failed");

    // Verify UART state captures guest serial output.
    assert_eq!(
        uart.get_buffer(),
        b"OK",
        "UART buffer did not match expected output"
    );
}

#[test]
fn test_phase5_long_mode_execution() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();

    let gpa: u64 = 0;
    let size: u64 = 2 * 1024 * 1024; // 2 MB guest address space.
    let mut guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();

    // Verify identity paging table setup.
    let pml4_addr = guest_mem
        .setup_identity_paging()
        .expect("Failed to setup paging");

    // 64-bit instruction: mov rax, 0x1122334455667788
    let payload: &[u8] = &[
        0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0xF4,
    ];
    let entry_point: u64 = 0x1000;
    guest_mem
        .write_bytes(entry_point, payload)
        .expect("Failed to write payload");

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    // Validate 64-bit transition and CR3 registration.
    vcpu.init_long_mode(entry_point, pml4_addr)
        .expect("Failed to init long mode");

    let event_loop = EventLoop::new(&vcpu);
    event_loop.run().expect("Event loop failed");

    let names = [WHvX64RegisterRax];
    let mut read_values = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&names, &mut read_values)
        .expect("Failed to get registers");

    assert_eq!(
        unsafe { read_values[0].Reg64 },
        0x1122334455667788,
        "Long mode execution failed: RAX value mismatch"
    );
}

#[test]
fn test_phase6_elf_loader() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 2 * 1024 * 1024).unwrap();
    let pml4_addr = guest_mem
        .setup_identity_paging()
        .expect("Failed to setup paging");

    // Requires `payload` crate to be compiled for `x86_64-unknown-none`.
    let payload_path = "target/x86_64-unknown-none/debug/payload";
    let elf_bytes = fs::read(payload_path)
        .expect("Failed to read ELF payload. Did you run `cargo build -p payload`?");

    let entry_point = load_elf(&elf_bytes, &mut guest_mem).expect("Failed to load ELF");

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();
    vcpu.init_long_mode(entry_point, pml4_addr)
        .expect("Failed to init long mode");

    // Map RSP to the top of 2MB to prevent stack faults during execution.
    let mut rsp_val = WHV_REGISTER_VALUE::default();
    rsp_val.Reg64 = 2 * 1024 * 1024;
    vcpu.set_registers(&[WHvX64RegisterRsp], &[rsp_val])
        .expect("Failed to set RSP");

    let mut uart = Uart::new();
    let event_loop = EventLoop::new(&vcpu);

    event_loop
        .run_with_uart(Some(&mut uart))
        .expect("Event loop failed");

    assert_eq!(
        uart.get_buffer(),
        b"K",
        "ELF payload did not output 'K' to UART"
    );
}

#[test]
fn test_phase7_page_fault_interception() {
    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();

    let gpa: u64 = 0;
    let size: u64 = 2 * 1024 * 1024;
    let mut guest_mem = GuestMemory::new(partition.as_raw(), gpa, size).unwrap();

    let pml4_addr = guest_mem
        .setup_identity_paging()
        .expect("Failed to setup paging");
    let entry_point: u64 = 0x10000;

    // Payload (64-bit Long Mode):
    // 48 8B 04 25 00 00 40 00 -> mov rax, qword ptr [0x400000]
    // Access unmapped memory to trigger #PF.
    let payload: &[u8] = &[0x48, 0x8B, 0x04, 0x25, 0x00, 0x00, 0x40, 0x00, 0xF4];
    guest_mem
        .write_bytes(entry_point, payload)
        .expect("Failed to write payload");

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();
    vcpu.init_long_mode(entry_point, pml4_addr)
        .expect("Failed to init long mode");

    let event_loop = EventLoop::new(&vcpu);
    let result = event_loop.run();

    // Verify WHPX intercepted and reported the Page Fault (#PF, E_FAIL).
    match result {
        Err(e) if e.code().0 == 0x80004005_u32 as i32 => (),
        other => panic!("Expected Page Fault VM exit (E_FAIL), but got: {:?}", other),
    }
}

#[test]
fn test_phase7_interrupt_injection() {
    use windows::Win32::System::Hypervisor::{
        WHvX64RegisterRax, WHvX64RegisterRsp, WHV_REGISTER_VALUE,
    };

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Partition::new().unwrap();
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 4096 * 4).unwrap();

    // 1. IVT (Interrupt Vector Table) for 16-bit Real Mode
    // Vector 0x20 offset: 0x20 * 4 = 0x80.
    // Format: [IP_Low, IP_High, CS_Low, CS_High] -> 0x0000:0x1000
    guest_mem
        .write_bytes(0x80, &[0x00, 0x10, 0x00, 0x00])
        .unwrap();

    // 2. Interrupt Handler at 0x1000
    // mov ax, 0x20 (B8 20 00)
    // hlt          (F4)
    let handler: &[u8] = &[0xB8, 0x20, 0x00, 0xF4];
    guest_mem.write_bytes(0x1000, handler).unwrap();

    // 3. Main Payload at 0x2000
    // int 0x20 (CD 20)
    // hlt      (F4)
    let main_code: &[u8] = &[0xCD, 0x20, 0xF4];
    guest_mem.write_bytes(0x2000, main_code).unwrap();

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();
    vcpu.init_state(0x2000).unwrap();

    // 4. Setup Stack (SP)
    // Hardware pushes FLAGS, CS, and IP during the INT execution.
    let mut rsp_val = WHV_REGISTER_VALUE::default();
    rsp_val.Reg64 = 0x3000;
    vcpu.set_registers(&[WHvX64RegisterRsp], &[rsp_val])
        .unwrap();

    // 5. Execute VM
    let event_loop = EventLoop::new(&vcpu);
    event_loop.run().unwrap();

    // 6. Verify Execution
    let mut rax_val = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&[WHvX64RegisterRax], &mut rax_val)
        .unwrap();

    assert_eq!(
        unsafe { rax_val[0].Reg64 } & 0xFFFF,
        0x20,
        "Software interrupt handler did not execute"
    );
}

#[test]
#[allow(unused_unsafe)]
fn test_phase9_ring3_execution() {
    use std::sync::Arc;
    use windows::Win32::System::Hypervisor::{WHvX64RegisterRax, WHV_REGISTER_VALUE};

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Arc::new(Partition::new().unwrap());
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 1024 * 1024).unwrap();

    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    // 1. Initialize Long Mode. Default state is Ring 0 with supervisor paging.
    vcpu.init_long_mode(0x2000, 0x3000).unwrap();

    // 2. Elevate paging structures to allow Ring 3 access (Bit 2: U/S = 1).
    guest_mem
        .write_bytes(0x3000, &[0x07, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x4000, &[0x07, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5000, &[0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // 3. Setup GDT with Ring 0 and Ring 3 descriptors.
    let gdt_entries: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 0x00: Null
        0x00, 0x00, 0x00, 0x00, 0x00, 0x9A, 0x20, 0x00, // 0x08: Ring 0 Code (DPL=0)
        0x00, 0x00, 0x00, 0x00, 0x00, 0x92, 0x00, 0x00, // 0x10: Ring 0 Data (DPL=0)
        0x00, 0x00, 0x00, 0x00, 0x00, 0xFA, 0x20, 0x00, // 0x18: Ring 3 Code (DPL=3)
        0x00, 0x00, 0x00, 0x00, 0x00, 0xF2, 0x00, 0x00, // 0x20: Ring 3 Data (DPL=3)
    ];
    guest_mem.write_bytes(0x1000, gdt_entries).unwrap();

    // Create a 10-byte GDTR descriptor in memory for the `lgdt` instruction.
    let gdtr_desc: &[u8] = &[
        0x27, 0x00, // Limit: 39 (5 entries * 8 bytes - 1)
        0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Base: 0x1000
    ];
    guest_mem.write_bytes(0x1500, gdtr_desc).unwrap();

    // 4. Bootstrap Payload (Ring 0): Loads GDT, constructs iretq stack frame, transitions to Ring 3.
    let bsp_code: &[u8] = &[
        0xB8, 0x00, 0x15, 0x00, 0x00, // mov eax, 0x1500
        0x0F, 0x01, 0x10, // lgdt [rax]
        0x48, 0xBC, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rsp, 0x10000
        0x6A, 0x23, // push 0x23 (SS: Ring 3 Data, RPL=3)
        0x68, 0x00, 0x00, 0x02, 0x00, // push 0x20000 (RSP)
        0x68, 0x02, 0x32, 0x00, 0x00, // push 0x3202 (RFLAGS: IOPL=3, IF=1)
        0x6A, 0x1B, // push 0x1B (CS: Ring 3 Code, RPL=3)
        0x68, 0x00, 0x80, 0x00, 0x00, // push 0x8000 (RIP)
        0x48, 0xCF, // iretq
    ];
    guest_mem.write_bytes(0x2000, bsp_code).unwrap();

    // 5. User-Space Payload (Ring 3)
    // mov eax, 0x33
    // out 0x16, al (Triggers VMExit intercepted by hypervisor)
    // jmp $
    let ring3_code: &[u8] = &[0xB8, 0x33, 0x00, 0x00, 0x00, 0xE6, 0x16, 0xEB, 0xFE];
    guest_mem.write_bytes(0x8000, ring3_code).unwrap();

    // Execute event loop.
    let event_loop = EventLoop::new(&vcpu);
    event_loop.run().unwrap();

    // Verify successful Ring 3 execution and VMExit routing.
    let mut rax = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&[WHvX64RegisterRax], &mut rax).unwrap();
    assert_eq!(unsafe { rax[0].Reg64 }, 0x33, "Ring 3 execution failed");
}

#[test]
#[allow(unused_unsafe)]
fn test_phase9_ring3_protection() {
    use std::sync::Arc;

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Arc::new(Partition::new().unwrap());
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 1024 * 1024).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    vcpu.init_long_mode(0x2000, 0x3000).unwrap();

    // Paging with User bit set (U/S = 1)
    guest_mem
        .write_bytes(0x3000, &[0x07, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x4000, &[0x07, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5000, &[0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // GDT & GDTR
    let gdt_entries: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9A, 0x20,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x92, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFA,
        0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF2, 0x00, 0x00,
    ];
    guest_mem.write_bytes(0x1000, gdt_entries).unwrap();
    let gdtr_desc: &[u8] = &[0x27, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    guest_mem.write_bytes(0x1500, gdtr_desc).unwrap();

    // Ring 0 Payload: Switch to Ring 3
    let bsp_code: &[u8] = &[
        0xB8, 0x00, 0x15, 0x00, 0x00, 0x0F, 0x01, 0x10, 0x48, 0xBC, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x6A, 0x23, 0x68, 0x00, 0x00, 0x02, 0x00, 0x68, 0x02, 0x32, 0x00, 0x00,
        0x6A, 0x1B, 0x68, 0x00, 0x80, 0x00, 0x00, 0x48, 0xCF,
    ];
    guest_mem.write_bytes(0x2000, bsp_code).unwrap();

    // Ring 3 Payload: Execute privileged instruction (hlt)
    let ring3_code: &[u8] = &[0xF4];
    guest_mem.write_bytes(0x8000, ring3_code).unwrap();

    let event_loop = EventLoop::new(&vcpu);
    let result = event_loop.run();

    // Assert that execution fails with E_FAIL (mapped from #GP by our EventLoop)
    assert!(
        result.is_err(),
        "Execution should fail due to privilege violation"
    );
    if let Err(e) = result {
        assert_eq!(e.code().0, 0x80004005_u32 as i32, "Expected E_FAIL for #GP");
    }
}

#[test]
#[allow(unused_unsafe)]
fn test_phase9_syscall_execution() {
    use std::sync::Arc;
    use windows::Win32::System::Hypervisor::{WHvX64RegisterRax, WHV_REGISTER_VALUE};

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Arc::new(Partition::new().unwrap());
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 1024 * 1024).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    vcpu.init_long_mode(0x2000, 0x3000).unwrap();

    // 1. Paging (U/S = 1)
    guest_mem
        .write_bytes(0x3000, &[0x07, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x4000, &[0x07, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5000, &[0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // 2. GDT & GDTR
    let gdt_entries: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9A, 0x20,
        0x00, // Ring 0 Code
        0x00, 0x00, 0x00, 0x00, 0x00, 0x92, 0x00, 0x00, // Ring 0 Data
        0x00, 0x00, 0x00, 0x00, 0x00, 0xF2, 0x00, 0x00, // Ring 3 Data
        0x00, 0x00, 0x00, 0x00, 0x00, 0xFA, 0x20, 0x00, // Ring 3 Code
    ];
    guest_mem.write_bytes(0x1000, gdt_entries).unwrap();
    let gdtr_desc: &[u8] = &[0x27, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    guest_mem.write_bytes(0x1500, gdtr_desc).unwrap();

    // 3. BSP Code: Native transition to Ring 3 via iretq
    let bsp_code: &[u8] = &[
        0xB8, 0x00, 0x15, 0x00, 0x00, // mov eax, 0x1500
        0x0F, 0x01, 0x10, // lgdt [rax]
        0x48, 0xBC, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rsp, 0x10000
        0x6A, 0x1B, // push SS (0x1B)
        0x68, 0x00, 0x00, 0x02, 0x00, // push RSP (0x20000)
        0x68, 0x02, 0x32, 0x00, 0x00, // push RFLAGS (0x3202)
        0x6A, 0x23, // push CS (0x23)
        0x68, 0x00, 0x80, 0x00, 0x00, // push RIP (0x8000)
        0x48, 0xCF, // iretq
    ];
    guest_mem.write_bytes(0x2000, bsp_code).unwrap();

    // 4. Syscall Handler
    let syscall_handler: &[u8] = &[
        0xBA, 0xF8, 0x03, // mov dx, 0x3f8
        0x4D, 0x85, 0xC0, // loop_start: test r8, r8
        0x74, 0x07, // jz done
        0xAC, // lodsb
        0xEE, // out dx, al
        0x49, 0xFF, 0xC8, // dec r8
        0xEB, 0xF4, // jmp loop_start
        0xB8, 0x42, 0x00, 0x00, 0x00, // done: mov eax, 0x42
        0x48, 0x0F, 0x07, // sysretq
    ];
    guest_mem.write_bytes(0x6000, syscall_handler).unwrap();

    // 5. Data Payload
    let text = b"FAANG Syscall\n";
    guest_mem.write_bytes(0x9000, text).unwrap();

    // 6. Ring 3 Code
    let ring3_code: &[u8] = &[
        0x48, 0xBE, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rsi, 0x9000
        0x49, 0xB8, 0x0E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mov r8, 14
        0x0F, 0x05, // syscall
        0xE6, 0x16, // out 0x16, al
        0xEB, 0xFE, // jmp $
    ];
    guest_mem.write_bytes(0x8000, ring3_code).unwrap();

    // 7. Configure Syscall MSRs
    vcpu.init_syscall(0x6000).unwrap();

    // 8. Execute Event Loop
    let mut uart = Uart::new();
    let event_loop = EventLoop::new(&vcpu);
    event_loop.run_with_uart(Some(&mut uart)).unwrap();

    // 9. Verify Syscall Return Code
    let mut rax = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&[WHvX64RegisterRax], &mut rax).unwrap();
    assert_eq!(unsafe { rax[0].Reg64 }, 0x42, "Syscall failed");
}

#[test]
#[allow(unused_unsafe)]
fn test_phase10_cpuid_signature() {
    use std::sync::Arc;
    use windows::Win32::System::Hypervisor::{
        WHvX64RegisterRax, WHvX64RegisterRbx, WHvX64RegisterRcx, WHvX64RegisterRdx,
        WHV_REGISTER_VALUE,
    };

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Arc::new(Partition::new().unwrap());
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 1024 * 1024).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    vcpu.init_long_mode(0x2000, 0x3000).unwrap();

    // Setup identity mapping for Long Mode
    guest_mem
        .write_bytes(0x3000, &[0x07, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x4000, &[0x07, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5000, &[0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // Guest code: request hypervisor signature via CPUID and halt
    let code: &[u8] = &[
        0xB8, 0x00, 0x00, 0x00, 0x40, // mov eax, 0x40000000
        0x0F, 0xA2, // cpuid
        0xF4, // hlt
    ];
    guest_mem.write_bytes(0x2000, code).unwrap();

    let event_loop = EventLoop::new(&vcpu);
    event_loop.run().unwrap();

    let mut regs = [WHV_REGISTER_VALUE::default(); 4];
    vcpu.get_registers(
        &[
            WHvX64RegisterRax,
            WHvX64RegisterRbx,
            WHvX64RegisterRcx,
            WHvX64RegisterRdx,
        ],
        &mut regs,
    )
    .unwrap();

    unsafe {
        assert_eq!(
            regs[0].Reg64, 0x40000001,
            "EAX should contain max supported leaf"
        );
        assert_eq!(regs[1].Reg64, 0x6C6F7349, "EBX should contain 'Isol'");
        assert_eq!(regs[2].Reg64, 0x4D566D75, "ECX should contain 'umVM'");
        assert_eq!(regs[3].Reg64, 0x00000000, "EDX should be 0");
    }
}

#[test]
#[allow(unused_unsafe)]
fn test_phase10_virtio_mmio_magic() {
    use std::fs::remove_file;
    use std::sync::Arc;
    use windows::Win32::System::Hypervisor::{WHvX64RegisterRax, WHV_REGISTER_VALUE};

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Arc::new(Partition::new().unwrap());
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 1024 * 1024).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    vcpu.init_long_mode(0x2000, 0x3000).unwrap();

    // Setup Paging: RAM (0x0 - 0x200000)
    guest_mem
        .write_bytes(0x3000, &[0x07, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x4000, &[0x07, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5000, &[0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // Setup Paging: MMIO VirtIO region via 4KB Page Table
    // PD index 128 (0x5400) -> points to PT at 0x6000
    guest_mem
        .write_bytes(0x5400, &[0x07, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    // PT at 0x6000, index 1 (for 0x10001000) -> points to physical 0x10001000
    guest_mem
        .write_bytes(0x6008, &[0x07, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // Guest Code: Read Magic Value from 0x10001000
    let code: &[u8] = &[
        0x48, 0xBF, 0x00, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, // mov rdi, 0x10001000
        0x8B, 0x07, // mov eax, [rdi]
        0xF4, // hlt
    ];
    guest_mem.write_bytes(0x2000, code).unwrap();

    let disk_path = "test_virtio_magic.img";
    let mut virtio = vmm_core::virtio::VirtioBlock::new(disk_path).unwrap();

    let event_loop = EventLoop::new(&vcpu);
    event_loop
        .run_with_devices(None, Some(&mut virtio), None)
        .unwrap();

    let mut regs = [WHV_REGISTER_VALUE::default()];
    vcpu.get_registers(&[WHvX64RegisterRax], &mut regs).unwrap();

    let _ = remove_file(disk_path);

    assert_eq!(
        unsafe { regs[0].Reg64 },
        0x74726976,
        "VirtIO MMIO magic value mismatch"
    );
}

#[test]
#[allow(unused_unsafe)]
fn test_phase10_virtio_block_rw() {
    use std::fs::{remove_file, File};
    use std::io::{Read, Write};
    use std::sync::Arc;

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let disk_path = "test_disk_full.img";
    {
        let mut f = File::create(disk_path).unwrap();
        let data = [0xAAu8; 512]; // Initial byte is 0xAA
        f.write_all(&data).unwrap();
    }

    let partition = Arc::new(Partition::new().unwrap());
    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 2 * 1024 * 1024).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    vcpu.init_long_mode(0x2000, 0x3000).unwrap();

    // Setup Paging
    guest_mem
        .write_bytes(0x3000, &[0x07, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x4000, &[0x07, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5000, &[0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5400, &[0x07, 0x60, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x6008, &[0x07, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // Desc 0, 1, 2: Read Request
    guest_mem
        .write_bytes(0x20000, &0x23000u64.to_le_bytes())
        .unwrap();
    guest_mem
        .write_bytes(0x20008, &16u32.to_le_bytes())
        .unwrap();
    guest_mem.write_bytes(0x2000C, &1u16.to_le_bytes()).unwrap(); // NEXT
    guest_mem.write_bytes(0x2000E, &1u16.to_le_bytes()).unwrap(); // next = 1

    guest_mem
        .write_bytes(0x20010, &0x24000u64.to_le_bytes())
        .unwrap();
    guest_mem
        .write_bytes(0x20018, &512u32.to_le_bytes())
        .unwrap();
    guest_mem.write_bytes(0x2001C, &3u16.to_le_bytes()).unwrap(); // NEXT | WRITE
    guest_mem.write_bytes(0x2001E, &2u16.to_le_bytes()).unwrap(); // next = 2

    guest_mem
        .write_bytes(0x20020, &0x25000u64.to_le_bytes())
        .unwrap();
    guest_mem.write_bytes(0x20028, &1u32.to_le_bytes()).unwrap();
    guest_mem.write_bytes(0x2002C, &2u16.to_le_bytes()).unwrap(); // WRITE
    guest_mem.write_bytes(0x2002E, &0u16.to_le_bytes()).unwrap(); // next = 0

    // Desc 3, 4, 5: Write Request
    guest_mem
        .write_bytes(0x20030, &0x26000u64.to_le_bytes())
        .unwrap();
    guest_mem
        .write_bytes(0x20038, &16u32.to_le_bytes())
        .unwrap();
    guest_mem.write_bytes(0x2003C, &1u16.to_le_bytes()).unwrap(); // NEXT
    guest_mem.write_bytes(0x2003E, &4u16.to_le_bytes()).unwrap(); // next = 4

    guest_mem
        .write_bytes(0x20040, &0x24000u64.to_le_bytes())
        .unwrap(); // Data buffer is the same
    guest_mem
        .write_bytes(0x20048, &512u32.to_le_bytes())
        .unwrap();
    guest_mem.write_bytes(0x2004C, &1u16.to_le_bytes()).unwrap(); // NEXT (Read from guest memory)
    guest_mem.write_bytes(0x2004E, &5u16.to_le_bytes()).unwrap(); // next = 5

    guest_mem
        .write_bytes(0x20050, &0x27000u64.to_le_bytes())
        .unwrap();
    guest_mem.write_bytes(0x20058, &1u32.to_le_bytes()).unwrap();
    guest_mem.write_bytes(0x2005C, &2u16.to_le_bytes()).unwrap(); // WRITE
    guest_mem.write_bytes(0x2005E, &0u16.to_le_bytes()).unwrap(); // next = 0

    // Initial Avail Ring (only Read request)
    guest_mem.write_bytes(0x21000, &0u16.to_le_bytes()).unwrap();
    guest_mem.write_bytes(0x21002, &1u16.to_le_bytes()).unwrap(); // idx = 1
    guest_mem.write_bytes(0x21004, &0u16.to_le_bytes()).unwrap(); // ring[0] = 0

    // Request Headers
    guest_mem.write_bytes(0x23000, &[0; 16]).unwrap(); // Read Header
    let mut write_hdr = [0u8; 16];
    write_hdr[0..4].copy_from_slice(&1u32.to_le_bytes()); // VIRTIO_BLK_T_OUT
    guest_mem.write_bytes(0x26000, &write_hdr).unwrap(); // Write Header

    // Guest Payload (Read -> Modify -> Write)
    let code: &[u8] = &[
        0x48, 0xBF, 0x50, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00,
        0x00, // mov rdi, 0x10001050 (QueueNotify)
        0x31, 0xC0, // xor eax, eax
        0x89, 0x07, // mov [rdi], eax  <-- Notify 1 (Read)
        // Modify data in buffer (0x24000)
        0x48, 0xBB, 0x00, 0x40, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, // mov rbx, 0x24000
        0x8A, 0x03, // mov al, [rbx]
        0x04, 0x01, // add al, 1 (0xAA -> 0xAB)
        0x88, 0x03, // mov [rbx], al
        // Update Avail Ring for Write Request
        0x66, 0xC7, 0x04, 0x25, 0x06, 0x10, 0x02, 0x00, 0x03,
        0x00, // mov word ptr [0x21006], 3 (ring[1] = desc 3)
        0x66, 0xC7, 0x04, 0x25, 0x02, 0x10, 0x02, 0x00, 0x02,
        0x00, // mov word ptr [0x21002], 2 (idx = 2)
        0x89, 0x07, // mov [rdi], eax  <-- Notify 2 (Write)
        0xF4, // hlt
    ];
    guest_mem.write_bytes(0x2000, code).unwrap();

    // Configure VirtIO Device
    let mut virtio = vmm_core::virtio::VirtioBlock::new(disk_path).unwrap();
    virtio.write_register(0x30, 0);
    virtio.write_register(0x80, 0x20000);
    virtio.write_register(0x84, 0);
    virtio.write_register(0x90, 0x21000);
    virtio.write_register(0x94, 0);
    virtio.write_register(0xA0, 0x22000);
    virtio.write_register(0xA4, 0);
    virtio.write_register(0x44, 1);

    let event_loop = EventLoop::new(&vcpu);
    event_loop
        .run_with_devices(None, Some(&mut virtio), Some(&mut guest_mem))
        .unwrap();

    // Verify disk image
    let mut f = File::open(disk_path).unwrap();
    let mut verified_data = vec![0u8; 512];
    f.read_exact(&mut verified_data).unwrap();

    let _ = remove_file(disk_path);

    assert_eq!(
        verified_data[0], 0xAB,
        "VirtIO Block cycle failed: File on host was not updated correctly."
    );
}

#[test]
#[allow(unused_unsafe)]
fn test_phase11_gdb_stub() {
    use std::ffi::c_void;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use windows::Win32::System::Hypervisor::{
        WHvPartitionPropertyCodeExceptionExitBitmap, WHvSetPartitionProperty,
        WHV_PARTITION_PROPERTY,
    };

    let present = is_hypervisor_present().unwrap_or(false);
    if !present {
        return;
    }

    let partition = Arc::new(Partition::new().unwrap());

    // Intercept #DB (1) and #BP (3) exceptions
    unsafe {
        let mut prop = WHV_PARTITION_PROPERTY::default();
        prop.ExceptionExitBitmap = (1 << 1) | (1 << 3);
        WHvSetPartitionProperty(
            partition.as_raw(),
            WHvPartitionPropertyCodeExceptionExitBitmap,
            &prop as *const _ as *const c_void,
            size_of::<WHV_PARTITION_PROPERTY>() as u32, // <-- Прибрано std::mem::
        )
        .unwrap();
    }

    let mut guest_mem = GuestMemory::new(partition.as_raw(), 0, 2 * 1024 * 1024).unwrap();
    let vcpu = Vcpu::new(partition.as_raw(), 0).unwrap();

    vcpu.init_long_mode(0x2000, 0x3000).unwrap();

    // Setup Paging (identity map first 2MB) - FIX for Exit Reason 0x4
    guest_mem
        .write_bytes(0x3000, &[0x07, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x4000, &[0x07, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();
    guest_mem
        .write_bytes(0x5000, &[0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .unwrap();

    // Payload: mov rax, 0x1122334455667788; int 3; hlt
    let code: &[u8] = &[
        0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // mov rax, ...
        0xCC, // int 3
        0xF4, // hlt
    ];
    guest_mem.write_bytes(0x2000, code).unwrap();

    let client_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(500));

        let mut stream =
            TcpStream::connect("127.0.0.1:9002").expect("Failed to connect to GDB stub");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let read_packet = |s: &mut TcpStream| -> String {
            let mut buf = [0u8; 1];
            let mut packet = String::new();
            let mut in_packet = false;
            loop {
                s.read_exact(&mut buf)
                    .expect("GDB Client: Failed to read from server");
                let c = buf[0] as char;
                if c == '$' {
                    in_packet = true;
                    packet.clear();
                } else if c == '#' && in_packet {
                    let mut csum = [0u8; 2];
                    s.read_exact(&mut csum).unwrap();
                    s.write_all(b"+").unwrap();
                    return packet;
                } else if in_packet {
                    packet.push(c);
                }
            }
        };

        let write_packet = |s: &mut TcpStream, data: &str| {
            let mut checksum = 0u8;
            for byte in data.bytes() {
                checksum = checksum.wrapping_add(byte);
            }
            let packet_str = format!("${}#{:02x}", data, checksum);
            s.write_all(packet_str.as_bytes()).unwrap();
            let mut ack = [0u8; 1];
            let _ = s.read_exact(&mut ack);
        };

        assert_eq!(read_packet(&mut stream), "S05");
        write_packet(&mut stream, "c");
        assert_eq!(read_packet(&mut stream), "S05");
        write_packet(&mut stream, "g");
        let reg_data = read_packet(&mut stream);

        assert!(
            reg_data.starts_with("8877665544332211"),
            "RAX value mismatch in GDB stub"
        );
        write_packet(&mut stream, "c");
    });

    let mut gdb_server = vmm_core::gdb::GdbServer::new(9002).unwrap();
    gdb_server.wait_for_connection().unwrap();

    let event_loop = EventLoop::new(&vcpu);
    let _ = event_loop.run_with_all(None, None, None, Some(&mut gdb_server));

    client_thread.join().expect("GDB Client thread panicked");
}
