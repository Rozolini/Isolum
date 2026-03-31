use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

/// Minimal GDB Remote Serial Protocol (RSP) server.
pub struct GdbServer {
    listener: TcpListener,
    stream: Option<TcpStream>,
}

impl GdbServer {
    /// Binds the TCP listener to the given port on localhost.
    pub fn new(port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        Ok(Self {
            listener,
            stream: None,
        })
    }

    /// Blocks until a GDB client connects.
    /// Sets a 500ms read timeout to prevent indefinite blocking during packet reads.
    pub fn wait_for_connection(&mut self) -> std::io::Result<()> {
        let (stream, _) = self.listener.accept()?;
        stream.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
        self.stream = Some(stream);
        Ok(())
    }

    /// Reads and decodes a GDB packet.
    /// Handles '$...#xx' framing and transmits acknowledgments ('+').
    pub fn read_packet(&mut self) -> std::io::Result<String> {
        let stream = self.stream.as_mut().expect("No active connection");
        let mut buf = [0u8; 1];
        let mut packet = String::new();
        let mut in_packet = false;

        loop {
            stream.read_exact(&mut buf)?;
            let c = buf[0] as char;

            if c == '$' {
                in_packet = true;
                packet.clear();
            } else if c == '#' && in_packet {
                // Read the 2-byte checksum.
                let mut csum = [0u8; 2];
                stream.read_exact(&mut csum)?;

                // Acknowledge packet reception.
                stream.write_all(b"+")?;
                return Ok(packet);
            } else if in_packet {
                packet.push(c);
            } else if c == '\x03' {
                // Intercept Ctrl-C (interrupt) from the GDB client.
                return Ok("vCtrlC".to_string());
            }
        }
    }

    /// Encodes and transmits a GDB packet.
    /// Automatically computes and appends the checksum.
    pub fn write_packet(&mut self, data: &str) -> std::io::Result<()> {
        let stream = self.stream.as_mut().expect("No active connection");

        let mut checksum = 0u8;
        for byte in data.bytes() {
            checksum = checksum.wrapping_add(byte);
        }

        let packet = format!("${}#{:02x}", data, checksum);
        stream.write_all(packet.as_bytes())?;

        // Consume the client's acknowledgment ('+').
        let mut ack = [0u8; 1];
        let _ = stream.read_exact(&mut ack);

        Ok(())
    }
}
