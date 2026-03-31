#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

// Entry point for the bare-metal payload.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        let mut result: u8;

        // Compute 0x40 + 0x0B = 0x4B ('K').
        asm!(
        "add al, bl",
        inout("al") 0x40u8 => result,
        in("bl") 0x0Bu8,
        );

        // Output the result to COM1 UART port (0x3F8).
        asm!(
        "out dx, al",
        in("dx") 0x3F8u16,
        in("al") result,
        );

        // Halt CPU execution.
        asm!("hlt");
    }
    loop {}
}

// Minimal panic handler. Spins indefinitely.
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
