use std::ffi::c_void;
use whpx_bindings::api::{
    create_virtual_processor, get_virtual_processor_registers, run_virtual_processor,
    set_virtual_processor_registers,
};
use windows::Win32::System::Hypervisor::{
    WHvX64RegisterCr0, WHvX64RegisterCr3, WHvX64RegisterCr4, WHvX64RegisterCs, WHvX64RegisterDs,
    WHvX64RegisterEfer, WHvX64RegisterEs, WHvX64RegisterFs, WHvX64RegisterGdtr, WHvX64RegisterGs,
    WHvX64RegisterIdtr, WHvX64RegisterLdtr, WHvX64RegisterRflags, WHvX64RegisterRip,
    WHvX64RegisterSs, WHvX64RegisterTr, WHV_PARTITION_HANDLE, WHV_REGISTER_NAME,
    WHV_REGISTER_VALUE, WHV_RUN_VP_EXIT_CONTEXT,
};

/// Represents a virtual CPU within a partition.
pub struct Vcpu {
    partition_handle: WHV_PARTITION_HANDLE,
    vp_index: u32,
}

impl Vcpu {
    /// Creates a new virtual CPU for the specified partition.
    pub fn new(
        partition_handle: WHV_PARTITION_HANDLE,
        vp_index: u32,
    ) -> Result<Self, windows::core::Error> {
        create_virtual_processor(partition_handle, vp_index)?;

        Ok(Self {
            partition_handle,
            vp_index,
        })
    }

    /// Initializes the fundamental execution state for Real Mode.
    #[allow(unused_unsafe)]
    pub fn init_state(&self, entry_point: u64) -> Result<(), windows::core::Error> {
        let names = [WHvX64RegisterRip, WHvX64RegisterRflags, WHvX64RegisterCs];
        let mut values = [WHV_REGISTER_VALUE::default(); 3];

        unsafe {
            values[0].Reg64 = entry_point;
            values[1].Reg64 = 0x02;

            values[2].Segment.Base = 0;
            values[2].Segment.Limit = 0xFFFF;
            values[2].Segment.Selector = 0;
            values[2].Segment.Anonymous.Attributes = 0x009B;
        }

        self.set_registers(&names, &values)
    }

    /// Initializes the vCPU for 64-bit Long Mode execution.
    #[allow(unused_unsafe)]
    pub fn init_long_mode(
        &self,
        entry_point: u64,
        pml4_addr: u64,
    ) -> Result<(), windows::core::Error> {
        let mut names = Vec::with_capacity(16);
        let mut values = Vec::with_capacity(16);

        unsafe {
            // 1. CR4: Enable Physical Address Extension (PAE).
            names.push(WHvX64RegisterCr4);
            let mut cr4 = WHV_REGISTER_VALUE::default();
            cr4.Reg64 = 0x20;
            values.push(cr4);

            // 2. CR3: Set PML4 base address.
            names.push(WHvX64RegisterCr3);
            let mut cr3 = WHV_REGISTER_VALUE::default();
            cr3.Reg64 = pml4_addr;
            values.push(cr3);

            // 3. EFER: Enable Long Mode (LME) and Long Mode Active (LMA).
            names.push(WHvX64RegisterEfer);
            let mut efer = WHV_REGISTER_VALUE::default();
            efer.Reg64 = 0x500; // 0x100 (LME) | 0x400 (LMA)
            values.push(efer);

            // 4. CR0: Enable Paging, Protection, and Numeric Error (PG | NE | ET | PE).
            names.push(WHvX64RegisterCr0);
            let mut cr0 = WHV_REGISTER_VALUE::default();
            cr0.Reg64 = 0x80000031; // PG=bit 31, NE=bit 5, ET=bit 4, PE=bit 0
            values.push(cr0);

            // 5. Code Segment: 64-bit.
            names.push(WHvX64RegisterCs);
            let mut cs_val = WHV_REGISTER_VALUE::default();
            cs_val.Segment.Base = 0;
            cs_val.Segment.Limit = 0xFFFFFFFF;
            cs_val.Segment.Selector = 0x08;
            cs_val.Segment.Anonymous.Attributes = 0xA09B; // G=1, L=1, P=1, Type=Execute/Read
            values.push(cs_val);

            // 6. Data Segments.
            for reg in [
                WHvX64RegisterDs,
                WHvX64RegisterEs,
                WHvX64RegisterSs,
                WHvX64RegisterFs,
                WHvX64RegisterGs,
            ] {
                names.push(reg);
                let mut ds_val = WHV_REGISTER_VALUE::default();
                ds_val.Segment.Base = 0;
                ds_val.Segment.Limit = 0xFFFFFFFF;
                ds_val.Segment.Selector = 0x10;
                ds_val.Segment.Anonymous.Attributes = 0xC093; // G=1, D/B=1, P=1, Type=Read/Write
                values.push(ds_val);
            }

            // 7. System Segments: TR and LDTR.
            names.push(WHvX64RegisterTr);
            let mut tr_val = WHV_REGISTER_VALUE::default();
            tr_val.Segment.Base = 0;
            tr_val.Segment.Limit = 0xFFFF;
            tr_val.Segment.Selector = 0x18;
            tr_val.Segment.Anonymous.Attributes = 0x008B; // P=1, Type=Busy TSS
            values.push(tr_val);

            names.push(WHvX64RegisterLdtr);
            let mut ldtr_val = WHV_REGISTER_VALUE::default();
            ldtr_val.Segment.Base = 0;
            ldtr_val.Segment.Limit = 0xFFFF;
            ldtr_val.Segment.Selector = 0x00;
            ldtr_val.Segment.Anonymous.Attributes = 0x0082; // P=1, Type=LDT
            values.push(ldtr_val);

            // 8. Global and Interrupt Descriptor Tables.
            names.push(WHvX64RegisterGdtr);
            let mut gdtr = WHV_REGISTER_VALUE::default();
            gdtr.Table.Base = 0;
            gdtr.Table.Limit = 0xFFFF;
            values.push(gdtr);

            names.push(WHvX64RegisterIdtr);
            let mut idtr = WHV_REGISTER_VALUE::default();
            idtr.Table.Base = 0;
            idtr.Table.Limit = 0xFFFF;
            values.push(idtr);

            // 9. RFLAGS and RIP.
            names.push(WHvX64RegisterRflags);
            let mut rflags = WHV_REGISTER_VALUE::default();
            rflags.Reg64 = 0x202;
            values.push(rflags);

            names.push(WHvX64RegisterRip);
            let mut rip = WHV_REGISTER_VALUE::default();
            rip.Reg64 = entry_point;
            values.push(rip);
        }

        self.set_registers(&names, &values)
    }

    pub fn set_registers(
        &self,
        names: &[WHV_REGISTER_NAME],
        values: &[WHV_REGISTER_VALUE],
    ) -> Result<(), windows::core::Error> {
        if names.len() != values.len() {
            panic!("Names and values arrays must have the same length");
        }

        unsafe {
            set_virtual_processor_registers(
                self.partition_handle,
                self.vp_index,
                names.as_ptr(),
                names.len() as u32,
                values.as_ptr(),
            )
        }
    }

    pub fn get_registers(
        &self,
        names: &[WHV_REGISTER_NAME],
        values: &mut [WHV_REGISTER_VALUE],
    ) -> Result<(), windows::core::Error> {
        if names.len() != values.len() {
            panic!("Names and values arrays must have the same length");
        }

        unsafe {
            get_virtual_processor_registers(
                self.partition_handle,
                self.vp_index,
                names.as_ptr(),
                names.len() as u32,
                values.as_mut_ptr(),
            )
        }
    }

    pub fn run(&self) -> Result<WHV_RUN_VP_EXIT_CONTEXT, windows::core::Error> {
        let mut exit_context = WHV_RUN_VP_EXIT_CONTEXT::default();

        unsafe {
            run_virtual_processor(
                self.partition_handle,
                self.vp_index,
                &mut exit_context as *mut _ as *mut c_void,
                size_of::<WHV_RUN_VP_EXIT_CONTEXT>() as u32,
            )?;
        }

        Ok(exit_context)
    }

    pub fn inject_interrupt(&self, vector: u8) -> Result<(), windows::core::Error> {
        use windows::Win32::System::Hypervisor::{
            WHvRegisterPendingInterruption, WHV_REGISTER_VALUE,
        };

        let mut reg = WHV_REGISTER_VALUE::default();

        // WHV_X64_PENDING_INTERRUPTION_REGISTER:
        // [0]      : InterruptionPending = 1
        // [1..3]   : InterruptionType = 4 (Software Interrupt)
        // [4]      : DeliverErrorCode = 0
        // [5..8]   : InstructionLength = 2 (Required by VT-x for SW interrupts)
        // [16..31] : InterruptionVector = vector
        reg.Reg64 = 1 | (4 << 1) | (2 << 5) | ((vector as u64) << 16);

        self.set_registers(&[WHvRegisterPendingInterruption], &[reg])
    }

    /// Initializes MSRs required for syscall/sysret instructions.
    #[allow(unused_unsafe)]
    pub fn init_syscall(&self, handler_addr: u64) -> Result<(), windows::core::Error> {
        use windows::Win32::System::Hypervisor::{
            WHvX64RegisterEfer, WHvX64RegisterLstar, WHvX64RegisterSfmask, WHvX64RegisterStar,
            WHV_REGISTER_VALUE,
        };

        let mut efer = [WHV_REGISTER_VALUE::default()];
        self.get_registers(&[WHvX64RegisterEfer], &mut efer)?;
        unsafe {
            efer[0].Reg64 |= 1;
        } // Enable SCE (System Call Extensions)

        let mut star = WHV_REGISTER_VALUE::default();
        // Sysret CS=0x10, Syscall CS=0x08
        unsafe {
            star.Reg64 = 0x0010000800000000;
        }

        let mut lstar = WHV_REGISTER_VALUE::default();
        unsafe {
            lstar.Reg64 = handler_addr;
        }

        let mut sfmask = WHV_REGISTER_VALUE::default();
        // Clear IF (Interrupt Flag) on syscall
        unsafe {
            sfmask.Reg64 = 0x0200;
        }

        self.set_registers(
            &[
                WHvX64RegisterEfer,
                WHvX64RegisterStar,
                WHvX64RegisterLstar,
                WHvX64RegisterSfmask,
            ],
            &[efer[0], star, lstar, sfmask],
        )
    }
}

// WHPX vCPU handles can be safely moved across threads.
unsafe impl Send for Vcpu {}
unsafe impl Sync for Vcpu {}
