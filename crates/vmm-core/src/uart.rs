use std::io::{self, Write};

// Standard Base I/O Address for COM1.
pub const COM1_PORT: u16 = 0x3F8;

/// Emulates a minimal 16550A UART device (COM1) for guest telemetry.
pub struct Uart {
    buffer: Vec<u8>,
}

impl Uart {
    /// Initializes a new UART emulator instance.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Processes Port I/O writes targeting the UART data register.
    pub fn write(&mut self, port: u16, data: u8) {
        if port == COM1_PORT {
            self.buffer.push(data);

            // Flush the buffer to the host standard output on line endings.
            if data == b'\n' {
                self.flush();
            }
        }
    }

    /// Exposes the internal buffer for validation in integration tests.
    pub fn get_buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Decodes and flushes the buffered guest output to the host stdout stream.
    pub fn flush(&mut self) {
        if !self.buffer.is_empty() {
            if let Ok(text) = std::str::from_utf8(&self.buffer) {
                print!("{}", text);
                let _ = io::stdout().flush();
            }
            self.buffer.clear();
        }
    }
}

impl Default for Uart {
    fn default() -> Self {
        Self::new()
    }
}
