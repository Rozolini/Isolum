use crate::memory::GuestMemory;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

// Define standard VirtIO MMIO base address and region size.
pub const VIRTIO_MMIO_BASE: u64 = 0x10001000;
pub const VIRTIO_MMIO_SIZE: u64 = 0x1000;

// VirtIO Block request types.
pub const VIRTIO_BLK_T_IN: u32 = 0; // Read from block device
pub const VIRTIO_BLK_T_OUT: u32 = 1; // Write to block device

/// Standard Virtqueue Descriptor structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

/// Emulates a VirtIO Block device backed by a host file via MMIO.
pub struct VirtioBlock {
    file: File,
    status: u32,
    queue_sel: u32,
    queue_desc: u64,
    queue_avail: u64,
    queue_used: u64,
    queue_ready: u32,
    last_avail_idx: u16,
    pub notify_pending: bool,
}

impl VirtioBlock {
    /// Initializes the block device using the specified host file path.
    pub fn new(file_path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(file_path)?;
        Ok(Self {
            file,
            status: 0,
            queue_sel: 0,
            queue_desc: 0,
            queue_avail: 0,
            queue_used: 0,
            queue_ready: 0,
            last_avail_idx: 0,
            notify_pending: false,
        })
    }

    /// Handles MMIO read requests from the guest for device configuration.
    pub fn read_register(&self, offset: u64) -> u32 {
        match offset {
            0x000 => 0x74726976,       // Magic Value ("virt")
            0x004 => 2,                // Device Version (2 = VirtIO 1.0)
            0x008 => 2,                // Device ID (2 = Block Device)
            0x00c => 0x554D4551,       // Vendor ID ("QEMU" used for compatibility)
            0x010 => 0,                // Device Features (None currently exposed)
            0x034 => 256,              // QueueNumMax (Max supported queue size)
            0x044 => self.queue_ready, // QueueReady status
            0x070 => self.status,      // DeviceStatus
            _ => 0,
        }
    }

    /// Handles MMIO write requests from the guest to configure the device.
    pub fn write_register(&mut self, offset: u64, value: u32) {
        match offset {
            0x030 => self.queue_sel = value,   // Select virtqueue index
            0x044 => self.queue_ready = value, // Enable/Disable virtqueue
            0x070 => self.status = value,      // Update device status
            // Configure Descriptor Table address (64-bit split across two 32-bit registers)
            0x080 => self.queue_desc = (self.queue_desc & 0xFFFFFFFF00000000) | (value as u64),
            0x084 => self.queue_desc = (self.queue_desc & 0xFFFFFFFF) | ((value as u64) << 32),
            // Configure Available Ring address
            0x090 => self.queue_avail = (self.queue_avail & 0xFFFFFFFF00000000) | (value as u64),
            0x094 => self.queue_avail = (self.queue_avail & 0xFFFFFFFF) | ((value as u64) << 32),
            // Configure Used Ring address
            0x0a0 => self.queue_used = (self.queue_used & 0xFFFFFFFF00000000) | (value as u64),
            0x0a4 => self.queue_used = (self.queue_used & 0xFFFFFFFF) | ((value as u64) << 32),
            // QueueNotify: Guest signals that new buffers are available
            0x050 => self.notify_pending = true,
            _ => {}
        }
    }

    /// Processes pending I/O requests in the virtqueue.
    pub fn process_queues(&mut self, mem: &mut GuestMemory) {
        if !self.notify_pending || self.queue_ready == 0 {
            return;
        }
        self.notify_pending = false;

        // Read the current available index from the Available Ring
        let mut idx_buf = [0u8; 2];
        mem.read_bytes(self.queue_avail + 2, &mut idx_buf).unwrap();
        let avail_idx = u16::from_le_bytes(idx_buf);

        // Process all new requests between last_avail_idx and avail_idx
        while self.last_avail_idx != avail_idx {
            let ring_offset = 4 + (self.last_avail_idx % 256) * 2;
            let mut desc_idx_buf = [0u8; 2];
            mem.read_bytes(self.queue_avail + ring_offset as u64, &mut desc_idx_buf)
                .unwrap();
            let head_idx = u16::from_le_bytes(desc_idx_buf);

            // Traverse the descriptor chain (Header -> Data -> Status)
            let desc1 = self.read_desc(mem, head_idx);
            let desc2 = self.read_desc(mem, desc1.next);
            let desc3 = self.read_desc(mem, desc2.next);

            // Parse the request header (virtio_blk_req)
            let mut req_buf = [0u8; 16];
            mem.read_bytes(desc1.addr, &mut req_buf).unwrap();
            let type_ = u32::from_le_bytes(req_buf[0..4].try_into().unwrap());
            let sector = u64::from_le_bytes(req_buf[8..16].try_into().unwrap());

            // Calculate host file offset based on 512-byte sectors
            let offset = sector * 512;
            self.file.seek(SeekFrom::Start(offset)).unwrap();

            if type_ == VIRTIO_BLK_T_OUT {
                // Handle write request: Read data from guest memory, write to host file
                let mut data = vec![0u8; desc2.len as usize];
                mem.read_bytes(desc2.addr, &mut data).unwrap();
                self.file.write_all(&data).unwrap();
            } else if type_ == VIRTIO_BLK_T_IN {
                // Handle read request: Read data from host file, write to guest memory
                let mut data = vec![0u8; desc2.len as usize];
                let bytes_read = self.file.read(&mut data).unwrap_or(0);
                mem.write_bytes(desc2.addr, &data[..bytes_read]).unwrap();
            }

            // Write operation status back to guest (0 = VIRTIO_BLK_S_OK)
            mem.write_bytes(desc3.addr, &[0]).unwrap();

            // Update the Used Ring to notify the guest of completion
            let used_idx_addr = self.queue_used + 2;
            let mut used_idx_buf = [0u8; 2];
            mem.read_bytes(used_idx_addr, &mut used_idx_buf).unwrap();
            let used_idx = u16::from_le_bytes(used_idx_buf);

            let used_ring_offset = 4 + (used_idx % 256) * 8;

            let head_idx_bytes = (head_idx as u32).to_le_bytes();
            mem.write_bytes(self.queue_used + used_ring_offset as u64, &head_idx_bytes)
                .unwrap();

            let len_bytes = desc2.len.to_le_bytes();
            mem.write_bytes(self.queue_used + used_ring_offset as u64 + 4, &len_bytes)
                .unwrap();

            // Increment and write the new Used Ring index
            let new_used_idx = used_idx.wrapping_add(1);
            let new_used_idx_bytes = new_used_idx.to_le_bytes();
            mem.write_bytes(used_idx_addr, &new_used_idx_bytes).unwrap();

            self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
        }
    }

    /// Helper to read a VirtqDesc struct from guest memory.
    fn read_desc(&self, mem: &GuestMemory, idx: u16) -> VirtqDesc {
        let addr = self.queue_desc + (idx as u64) * 16;
        let mut buf = [0u8; 16];
        mem.read_bytes(addr, &mut buf).unwrap();
        VirtqDesc {
            addr: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
            len: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
            flags: u16::from_le_bytes(buf[12..14].try_into().unwrap()),
            next: u16::from_le_bytes(buf[14..16].try_into().unwrap()),
        }
    }
}
