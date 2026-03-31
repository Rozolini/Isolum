use crate::gdb::GdbServer;
use crate::memory::GuestMemory;
use crate::uart::Uart;
use crate::vcpu::Vcpu;
use crate::virtio::{VirtioBlock, VIRTIO_MMIO_BASE, VIRTIO_MMIO_SIZE};
use windows::Win32::System::Hypervisor::{
    WHvRunVpExitReasonException, WHvRunVpExitReasonMemoryAccess, WHvRunVpExitReasonX64Cpuid,
    WHvRunVpExitReasonX64Halt, WHvRunVpExitReasonX64IoPortAccess, WHvX64RegisterR10,
    WHvX64RegisterR11, WHvX64RegisterR12, WHvX64RegisterR13, WHvX64RegisterR14, WHvX64RegisterR15,
    WHvX64RegisterR8, WHvX64RegisterR9, WHvX64RegisterRax, WHvX64RegisterRbp, WHvX64RegisterRbx,
    WHvX64RegisterRcx, WHvX64RegisterRdi, WHvX64RegisterRdx, WHvX64RegisterRflags,
    WHvX64RegisterRip, WHvX64RegisterRsi, WHvX64RegisterRsp, WHV_REGISTER_VALUE,
};

/// Primary execution loop for a virtual CPU.
/// Handles VMExits and routes hardware/IO events to emulated devices.
pub struct EventLoop<'a> {
    vcpu: &'a Vcpu,
}

impl<'a> EventLoop<'a> {
    pub fn new(vcpu: &'a Vcpu) -> Self {
        Self { vcpu }
    }

    /// Executes the vCPU without attached devices.
    pub fn run(&self) -> Result<(), windows::core::Error> {
        self.run_with_all(None, None, None, None)
    }

    /// Executes the vCPU with UART emulation enabled.
    pub fn run_with_uart(&self, uart: Option<&mut Uart>) -> Result<(), windows::core::Error> {
        self.run_with_all(uart, None, None, None)
    }

    /// Executes the vCPU with UART and VirtIO Block devices.
    pub fn run_with_devices(
        &self,
        uart: Option<&mut Uart>,
        virtio: Option<&mut VirtioBlock>,
        mem: Option<&mut GuestMemory>,
    ) -> Result<(), windows::core::Error> {
        self.run_with_all(uart, virtio, mem, None)
    }

    /// Core execution loop. Handles state transitions and dispatches VMExits.
    pub fn run_with_all(
        &self,
        mut uart: Option<&mut Uart>,
        mut virtio: Option<&mut VirtioBlock>,
        mut mem: Option<&mut GuestMemory>,
        mut gdb: Option<&mut GdbServer>,
    ) -> Result<(), windows::core::Error> {
        // Halt execution to sync with GDB client before proceeding.
        if let Some(ref mut g) = gdb {
            self.handle_gdb(g)?;
        }

        let mut step_count = 0;

        loop {
            step_count += 1;
            // Prevent infinite loops in testing/bare-metal scenarios.
            if step_count > 10000 {
                let mut rip_val = [WHV_REGISTER_VALUE::default()];
                let _ = self.vcpu.get_registers(&[WHvX64RegisterRip], &mut rip_val);
                panic!(
                    "VM stuck in an infinite loop! Exceeded 10000 exits. RIP: {:#X}",
                    unsafe { rip_val[0].Reg64 }
                );
            }

            // Transfer control to the guest. Blocks until a VMExit occurs.
            let exit_context = self.vcpu.run()?;

            #[allow(non_upper_case_globals)]
            match exit_context.ExitReason {
                WHvRunVpExitReasonX64Halt => {
                    // Guest executed HLT instruction.
                    break;
                }
                WHvRunVpExitReasonX64IoPortAccess => {
                    // Handle Port I/O (PIO) VMExit.
                    let io = unsafe { exit_context.Anonymous.IoPortAccess };

                    // Custom exit port for tests/graceful shutdown.
                    if io.PortNumber == 0x16 {
                        break;
                    }

                    let mut handled = false;
                    // Route to UART if within the 16550A port range.
                    if let Some(ref mut u) = uart {
                        if (0x3F8..=0x3FF).contains(&io.PortNumber) {
                            u.write(io.PortNumber, (io.Rax & 0xFF) as u8);
                            handled = true;
                        }
                    }

                    if !handled {
                        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                            0x80004002_u32 as i32,
                        )));
                    }

                    // Advance RIP past the IN/OUT instruction.
                    let mut rip_val = [WHV_REGISTER_VALUE::default()];
                    self.vcpu
                        .get_registers(&[WHvX64RegisterRip], &mut rip_val)?;

                    unsafe {
                        let len = if io.InstructionByteCount > 0 {
                            io.InstructionByteCount as u64
                        } else {
                            1
                        };
                        rip_val[0].Reg64 += len;
                    }

                    self.vcpu.set_registers(&[WHvX64RegisterRip], &rip_val)?;
                }
                WHvRunVpExitReasonMemoryAccess => {
                    // Handle Memory-Mapped I/O (MMIO) VMExit.
                    let access = unsafe { exit_context.Anonymous.MemoryAccess };
                    let gpa = access.Gpa;

                    // Route to VirtIO block device if within its MMIO region.
                    if (VIRTIO_MMIO_BASE..VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE).contains(&gpa) {
                        if let Some(ref mut dev) = virtio {
                            let offset = gpa - VIRTIO_MMIO_BASE;
                            // 0x89 is MOV reg/mem (write).
                            let is_write = access.InstructionBytes[0] == 0x89;

                            let mut rax = [WHV_REGISTER_VALUE::default()];
                            self.vcpu.get_registers(&[WHvX64RegisterRax], &mut rax)?;

                            if is_write {
                                dev.write_register(offset, unsafe { rax[0].Reg32 });
                                // Trigger virtqueue processing if memory is available.
                                if let Some(ref mut m) = mem {
                                    dev.process_queues(m);
                                }
                            } else {
                                let val = dev.read_register(offset);
                                unsafe {
                                    // Mask lower 32 bits and insert read value.
                                    rax[0].Reg64 =
                                        (rax[0].Reg64 & 0xFFFFFFFF00000000) | (val as u64);
                                }
                                self.vcpu.set_registers(&[WHvX64RegisterRax], &rax)?;
                            }
                        }
                    }

                    // Advance RIP past the memory access instruction.
                    let mut rip_val = [WHV_REGISTER_VALUE::default()];
                    self.vcpu
                        .get_registers(&[WHvX64RegisterRip], &mut rip_val)?;

                    unsafe {
                        let len = if (access.InstructionBytes[0] == 0x8B
                            || access.InstructionBytes[0] == 0x89)
                            && access.InstructionBytes[1] == 0x07
                        {
                            2
                        } else if access.InstructionByteCount > 0 {
                            access.InstructionByteCount as u64
                        } else {
                            if !(VIRTIO_MMIO_BASE..VIRTIO_MMIO_BASE + VIRTIO_MMIO_SIZE)
                                .contains(&gpa)
                            {
                                return Err(windows::core::Error::from_hresult(
                                    windows::core::HRESULT(0x80070005_u32 as i32),
                                ));
                            }
                            1
                        };
                        rip_val[0].Reg64 += len;
                    }

                    self.vcpu.set_registers(&[WHvX64RegisterRip], &rip_val)?;
                }
                WHvRunVpExitReasonException => {
                    // Handle hardware exceptions injected into the hypervisor.
                    let exception = unsafe { exit_context.Anonymous.VpException };

                    // Route #DB (1) and #BP (3) to GDB stub.
                    if exception.ExceptionType == 1 || exception.ExceptionType == 3 {
                        if let Some(ref mut g) = gdb {
                            self.handle_gdb(g)?;

                            // Advance RIP past 'int 3' (0xCC) to prevent re-execution.
                            if exception.ExceptionType == 3 {
                                let mut rip_val = [WHV_REGISTER_VALUE::default()];
                                self.vcpu
                                    .get_registers(&[WHvX64RegisterRip], &mut rip_val)?;
                                unsafe {
                                    rip_val[0].Reg64 += 1;
                                }
                                self.vcpu.set_registers(&[WHvX64RegisterRip], &rip_val)?;
                            }

                            continue;
                        } else {
                            panic!("Breakpoint hit but no GDB attached!");
                        }
                    }

                    // Propagate #PF (14), #DF (8), #GP (13) as HRESULT errors.
                    if exception.ExceptionType == 14
                        || exception.ExceptionType == 8
                        || exception.ExceptionType == 13
                    {
                        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                            0x80004005_u32 as i32,
                        )));
                    } else {
                        panic!(
                            "Unhandled VM Exception! Type: {}, ErrorCode: {:#X}, Parameter: {:#X}",
                            exception.ExceptionType,
                            exception.ErrorCode,
                            exception.ExceptionParameter
                        );
                    }
                }
                WHvRunVpExitReasonX64Cpuid => {
                    // Handle CPUID instruction interception.
                    let cpuid = unsafe { exit_context.Anonymous.CpuidAccess };
                    let leaf = cpuid.Rax as u32;

                    let mut eax = cpuid.DefaultResultRax;
                    let mut ebx = cpuid.DefaultResultRbx;
                    let mut ecx = cpuid.DefaultResultRcx;
                    let mut edx = cpuid.DefaultResultRdx;

                    // Inject custom hypervisor signature ("IsolumVM").
                    if leaf == 0x40000000 {
                        eax = 0x40000001;
                        ebx = 0x6C6F7349;
                        ecx = 0x4D566D75;
                        edx = 0x00000000;
                    } else if leaf == 1 {
                        // Set hypervisor present bit.
                        ecx |= 1 << 31;
                    }

                    let mut rip_val = [WHV_REGISTER_VALUE::default()];
                    self.vcpu
                        .get_registers(&[WHvX64RegisterRip], &mut rip_val)?;

                    let mut reg_values = [WHV_REGISTER_VALUE::default(); 5];
                    reg_values[0].Reg64 = eax;
                    reg_values[1].Reg64 = ebx;
                    reg_values[2].Reg64 = ecx;
                    reg_values[3].Reg64 = edx;
                    // Advance RIP past the 2-byte CPUID instruction.
                    reg_values[4].Reg64 = unsafe { rip_val[0].Reg64 } + 2;

                    self.vcpu.set_registers(
                        &[
                            WHvX64RegisterRax,
                            WHvX64RegisterRbx,
                            WHvX64RegisterRcx,
                            WHvX64RegisterRdx,
                            WHvX64RegisterRip,
                        ],
                        &reg_values,
                    )?;
                }
                _ => {
                    panic!(
                        "Unhandled VM Exit. Reason: {:#X}",
                        exit_context.ExitReason.0
                    );
                }
            }
        }

        Ok(())
    }

    /// Processes commands from the connected GDB client.
    fn handle_gdb(&self, gdb: &mut GdbServer) -> Result<(), windows::core::Error> {
        // Explicit type annotation for the closure to satisfy the compiler
        let _io_err = |_: std::io::Error| {
            windows::core::Error::from_hresult(windows::core::HRESULT(0x80004005_u32 as i32))
        };

        // Send initial stop reason (SIGTRAP)
        let _ = gdb.write_packet("S05");

        while let Ok(packet) = gdb.read_packet() {
            if packet.is_empty() {
                continue;
            }

            match packet.as_str() {
                "?" => {
                    let _ = gdb.write_packet("S05");
                }
                "g" => {
                    let reg_names = [
                        WHvX64RegisterRax,
                        WHvX64RegisterRbx,
                        WHvX64RegisterRcx,
                        WHvX64RegisterRdx,
                        WHvX64RegisterRsi,
                        WHvX64RegisterRdi,
                        WHvX64RegisterRbp,
                        WHvX64RegisterRsp,
                        WHvX64RegisterR8,
                        WHvX64RegisterR9,
                        WHvX64RegisterR10,
                        WHvX64RegisterR11,
                        WHvX64RegisterR12,
                        WHvX64RegisterR13,
                        WHvX64RegisterR14,
                        WHvX64RegisterR15,
                        WHvX64RegisterRip,
                        WHvX64RegisterRflags,
                    ];
                    let mut reg_values = [WHV_REGISTER_VALUE::default(); 18];

                    if self.vcpu.get_registers(&reg_names, &mut reg_values).is_ok() {
                        let mut reply = String::with_capacity(reg_values.len() * 16);
                        for val in &reg_values {
                            let bytes = unsafe { val.Reg64 }.to_le_bytes();
                            for b in bytes {
                                reply.push_str(&format!("{:02x}", b));
                            }
                        }
                        let _ = gdb.write_packet(&reply);
                    } else {
                        let _ = gdb.write_packet("E01");
                    }
                }
                "c" | "vCont;c" => {
                    break;
                }
                _ => {
                    // Send empty response for unsupported commands
                    let _ = gdb.write_packet("");
                }
            }
        }
        Ok(())
    }
}
