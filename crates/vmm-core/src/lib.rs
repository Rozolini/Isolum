// ELF binary parsing and segment mapping.
pub mod elf;

// Main VM execution loop and VMExit handling.
pub mod event_loop;

// Guest payload loading mechanisms (flat binary and ELF).
pub mod loader;

// Guest physical memory allocation and paging management.
pub mod memory;

// WHPX partition lifecycle and isolation boundaries.
pub mod partition;

// UART 16550A serial port emulation for telemetry.
pub mod uart;

// Virtual CPU state, registers, and execution control.
pub mod vcpu;

// VirtIO block device emulation via MMIO.
pub mod virtio;

// GDB Remote Serial Protocol (RSP) stub for debugging.
pub mod gdb;
