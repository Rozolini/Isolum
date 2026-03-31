# Isolum

Isolum is a lightweight, hardware-accelerated x86_64 hypervisor for Windows, built on top of the Windows Hypervisor Platform (WHPX) API. It provides a strictly isolated virtual machine environment designed to execute bare-metal payloads and ELF binaries directly on the host CPU.

## Architecture

The hypervisor is strictly modularized into components focused on zero-cost hardware virtualization and safe state management:

### 1. Virtual Machine Monitor (VMM) & Execution
* **WHPX Integration:** Manages partition lifecycles and Virtual Processor (vCPU) contexts via direct FFI bindings to `Win32_System_Hypervisor`.


* **State Initialization:** Automatically bootstraps the vCPU into 64-bit Long Mode, configuring control registers (CR0, CR3, CR4, EFER) and segment descriptors (GDT/IDT).


* **VMExit Routing:** Intercepts and routes hardware events, including CPUID instructions, Port I/O (PIO), Memory-Mapped I/O (MMIO), and hardware exceptions (#PF, #DF, #GP).

### 2. Memory Subsystem
* **Guest Physical Memory (GPA):** Allocates page-aligned host virtual memory via `VirtualAlloc` and maps it to the guest physical address space.


* **Identity Paging:** Implements automated 4-level page table generation (PML4 -> PDPT -> PD -> PT) to provide immediate 1:1 memory translation for guest payloads.

### 3. Device Emulation & Debugging
* **VirtIO Block Device (MMIO):** Emulates a standard VirtIO block storage interface, enabling the guest to read/write to host-backed image files via virtqueues.

 
* **UART 16550A (COM1):** Provides synchronous serial port emulation for guest telemetry and standard output interception.


* **GDB Remote Serial Protocol:** Implements a TCP-based RSP stub, allowing standard GDB clients to attach, set breakpoints (#BP), and inspect vCPU registers.

## Getting Started

### Prerequisites

* Rust toolchain (stable)


* Rust nightly (strictly for Miri UB verification)


* Windows 10/11 x86_64 target


* **Windows Hypervisor Platform** feature enabled in OS settings

### Installation

Clone the repository and build the workspace:

```bash
git clone https://github.com/Rozolini/Isolum.git
cd Isolum
cargo build --workspace
```

## Verification

Due to extensive FFI usage and direct memory manipulation, the project relies on strict automated verification.

### 1. Undefined Behavior Detection (Miri)

Validates memory provenance and unsafe blocks within the library components. 
Hardware-specific WHPX calls are bypassed during interpretation.

```bash
cargo +nightly miri test -p vmm-core --lib --all-features
```

### 2. Concurrency Testing (Loom)

Exhaustively simulates thread interleavings for shared state components to guarantee the absence of data races. Executed single-threaded to prevent WHPX partition collisions at the OS level.

```bash
$env:RUSTFLAGS="--cfg loom"; cargo test --release -- --test-threads=1
```

### 3. Concurrency & Integration Testing

Tests the hypervisor lifecycle. 
Due to WHPX partition isolation limits in CI environments, integration tests run single-threaded.

```bash
cargo test --all-targets --all-features -- --test-threads=1
```

## End-to-End Testing
To verify the entire virtualization pipeline, ELF loading, and I/O interception, run the E2E integration test:

```bash
# 1. Compile the bare-metal guest payload
cargo build -p payload --target x86_64-unknown-none

# 2. Execute the full system test (requires Administrator privileges)
cargo test --test e2e_test -- --nocapture
```

**What it tests:**

**1. Virtualization Setup:** Partition creation, memory allocation, and Long Mode initialization.

**2. ELF Loader:** Parses sections and maps the payload into guest memory.

**3. Instruction Execution:** Verifies native CPU execution of the guest payload and register state mutations.

**4. I/O Interception:** Confirms that guest `OUT` instructions are trapped, routed to the UART emulator, and correctly decoded.

## Design Considerations

* **Zero-Cost Execution:** Relies entirely on hardware virtualization (Intel VT-x / AMD-V via WHPX). The hypervisor only consumes cycles during explicit VMExits.


* **Unsafe Code Isolation:** `unsafe` blocks are strictly confined to OS-level FFI (`whpx-bindings`), raw pointer dereferencing for GPA mappings, and vCPU register state transitions.


* **Minimal Footprint:** The guest payload environment operates entirely in `#![no_std]`, requiring no external dependencies or host OS concepts.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.


