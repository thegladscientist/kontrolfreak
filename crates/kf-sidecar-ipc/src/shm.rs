//! Zero-copy file-backed shared-memory ring buffer (`memmap2`) with lock-free atomic
//! write/read sequence counters and `#[repr(C)] ShmFrameHeader` for transporting raw
//! 16-bit millimeter depth frames between the sidecar producer and host consumer.

use crate::protocol::{PixelFormat, ShmFrameHeader, SidecarError, SIDECAR_PROTOCOL_VERSION};
use memmap2::{MmapMut, MmapOptions};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const RING_MAGIC: u32 = 0x4B46_5242; // "KFRB"
const HEADER_ALIGN: usize = 64;

/// Fixed-layout atomic control block located at byte offset 0 of the shared-memory file.
#[repr(C, align(64))]
struct ShmRingHeader {
    magic: u32,
    protocol_version: u32,
    slot_count: u32,
    slot_stride: u32,
    max_payload_bytes: u32,
    width: u32,
    height: u32,
    _reserved: u32,
    write_seq: AtomicU64,
    read_seq: AtomicU64,
    dropped_frames: AtomicU64,
    producer_pid: AtomicU32,
    producer_alive: AtomicU32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShmRingHealth {
    pub slot_count: u32,
    pub write_seq: u64,
    pub read_seq: u64,
    pub dropped_frames: u64,
    pub utilization: f32,
    pub producer_pid: u32,
    pub producer_alive: bool,
}

const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

fn slot_layout(max_payload_bytes: usize) -> (usize, usize) {
    let payload_offset = align_up(size_of::<ShmFrameHeader>(), 8);
    let slot_stride = align_up(payload_offset + max_payload_bytes, HEADER_ALIGN);
    (payload_offset, slot_stride)
}

fn total_file_size(slot_count: u32, slot_stride: usize) -> usize {
    let ring_hdr_size = align_up(size_of::<ShmRingHeader>(), HEADER_ALIGN);
    ring_hdr_size + (slot_count as usize) * slot_stride
}

fn ring_header_ref(mmap: &MmapMut) -> &ShmRingHeader {
    debug_assert!(mmap.len() >= size_of::<ShmRingHeader>());
    debug_assert_eq!(mmap.as_ptr() as usize % align_of::<ShmRingHeader>(), 0);
    // SAFETY: `MmapMut` pages are page-aligned (>= 4096 bytes >= 64) and length is verified >= size_of::<ShmRingHeader>().
    unsafe { &*(mmap.as_ptr() as *const ShmRingHeader) }
}

/// Producer handle for writing raw 16-bit depth frames into the shared-memory ring buffer.
pub struct ShmRingWriter {
    path: PathBuf,
    mmap: MmapMut,
    slot_count: u32,
    slot_stride: usize,
    payload_offset: usize,
    max_payload_bytes: usize,
    ring_hdr_size: usize,
}

impl ShmRingWriter {
    /// Creates and initializes a new shared-memory ring buffer file on disk.
    pub fn create(
        path: impl AsRef<Path>,
        slot_count: u32,
        max_width: u32,
        max_height: u32,
    ) -> Result<Self, SidecarError> {
        let slot_count = slot_count.max(2);
        let max_payload_bytes = (max_width as usize)
            .saturating_mul(max_height as usize)
            .saturating_mul(size_of::<u16>())
            .max(4096);
        let (payload_offset, slot_stride) = slot_layout(max_payload_bytes);
        let ring_hdr_size = align_up(size_of::<ShmRingHeader>(), HEADER_ALIGN);
        let total_size = total_file_size(slot_count, slot_stride);

        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(&path_buf)?;
        file.set_len(total_size as u64)?;

        // SAFETY: File is sized to `total_size` and mapped exclusively for IPC ring initialization.
        let mut mmap = unsafe { MmapOptions::new().len(total_size).map_mut(&file)? };
        mmap.fill(0);

        {
            // SAFETY: Page-aligned buffer with verified size >= size_of::<ShmRingHeader>().
            let hdr = unsafe { &mut *(mmap.as_mut_ptr() as *mut ShmRingHeader) };
            hdr.magic = RING_MAGIC;
            hdr.protocol_version = SIDECAR_PROTOCOL_VERSION;
            hdr.slot_count = slot_count;
            hdr.slot_stride = slot_stride as u32;
            hdr.max_payload_bytes = max_payload_bytes as u32;
            hdr.width = max_width;
            hdr.height = max_height;
            hdr._reserved = 0;
            hdr.write_seq = AtomicU64::new(0);
            hdr.read_seq = AtomicU64::new(0);
            hdr.dropped_frames = AtomicU64::new(0);
            hdr.producer_pid = AtomicU32::new(std::process::id());
            hdr.producer_alive = AtomicU32::new(1);
        }
        mmap.flush()?;

        Ok(Self {
            path: path_buf,
            mmap,
            slot_count,
            slot_stride,
            payload_offset,
            max_payload_bytes,
            ring_hdr_size,
        })
    }

    /// Opens an existing initialized shared-memory ring buffer as the producer.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SidecarError> {
        let path_buf = path.as_ref().to_path_buf();
        let file = OpenOptions::new().read(true).write(true).open(&path_buf)?;
        let len = file.metadata()?.len() as usize;
        if len < size_of::<ShmRingHeader>() {
            return Err(SidecarError::Shm(
                "shared-memory file smaller than ring header".into(),
            ));
        }

        // SAFETY: File length verified; caller ensures file is not truncated while mapped.
        let mmap = unsafe { MmapOptions::new().len(len).map_mut(&file)? };
        let hdr = ring_header_ref(&mmap);
        if hdr.magic != RING_MAGIC {
            return Err(SidecarError::Shm(format!(
                "invalid ring buffer magic: 0x{:08X}",
                hdr.magic
            )));
        }
        if hdr.protocol_version != SIDECAR_PROTOCOL_VERSION {
            return Err(SidecarError::Shm(format!(
                "unsupported ring protocol version {}",
                hdr.protocol_version
            )));
        }

        let slot_count = hdr.slot_count;
        let slot_stride = hdr.slot_stride as usize;
        let max_payload_bytes = hdr.max_payload_bytes as usize;
        let (payload_offset, expected_stride) = slot_layout(max_payload_bytes);
        if slot_count < 2 || slot_stride != expected_stride {
            return Err(SidecarError::Shm("corrupted ring buffer geometry".into()));
        }
        let ring_hdr_size = align_up(size_of::<ShmRingHeader>(), HEADER_ALIGN);
        let needed = total_file_size(slot_count, slot_stride);
        if len < needed {
            return Err(SidecarError::Shm(
                "shared-memory file truncated below ring geometry".into(),
            ));
        }

        hdr.producer_pid.store(std::process::id(), Ordering::Release);
        hdr.producer_alive.store(1, Ordering::Release);

        Ok(Self {
            path: path_buf,
            mmap,
            slot_count,
            slot_stride,
            payload_offset,
            max_payload_bytes,
            ring_hdr_size,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a 16-bit depth frame (millimeters) directly into the next ring slot with zero serialization.
    pub fn write_depth_u16(
        &mut self,
        width: u32,
        height: u32,
        timestamp_ns: u64,
        depth_mm: &[u16],
    ) -> Result<u64, SidecarError> {
        let expected_pixels = (width as usize).saturating_mul(height as usize);
        if depth_mm.len() != expected_pixels {
            return Err(SidecarError::Shm(format!(
                "depth slice length {} != {}x{}",
                depth_mm.len(),
                width,
                height
            )));
        }
        let byte_len = expected_pixels * size_of::<u16>();
        if byte_len > self.max_payload_bytes {
            return Err(SidecarError::Shm(format!(
                "frame payload {}B exceeds ring capacity {}B",
                byte_len, self.max_payload_bytes
            )));
        }

        let (next_seq, slot_offset) = {
            let hdr = ring_header_ref(&self.mmap);
            let cur_write = hdr.write_seq.load(Ordering::Relaxed);
            let cur_read = hdr.read_seq.load(Ordering::Acquire);
            let next_seq = cur_write + 1;
            if next_seq.saturating_sub(cur_read) > self.slot_count as u64 {
                hdr.dropped_frames.fetch_add(1, Ordering::Relaxed);
            }
            let slot_idx = ((next_seq - 1) % self.slot_count as u64) as usize;
            (next_seq, self.ring_hdr_size + slot_idx * self.slot_stride)
        };

        let frame_hdr = ShmFrameHeader::new(
            next_seq,
            timestamp_ns,
            width,
            height,
            width * size_of::<u16>() as u32,
            PixelFormat::Depth16,
        );

        // Write ShmFrameHeader bytes
        let hdr_dst = &mut self.mmap[slot_offset..slot_offset + size_of::<ShmFrameHeader>()];
        // SAFETY: `ShmFrameHeader` is `#[repr(C)]` Copy with no padding invariants violated on byte copy.
        let hdr_src = unsafe {
            std::slice::from_raw_parts(
                (&frame_hdr as *const ShmFrameHeader) as *const u8,
                size_of::<ShmFrameHeader>(),
            )
        };
        hdr_dst.copy_from_slice(hdr_src);

        // Write raw little-endian u16 depth pixels directly into mapped slot
        let payload_start = slot_offset + self.payload_offset;
        let payload_dst = &mut self.mmap[payload_start..payload_start + byte_len];
        #[cfg(target_endian = "little")]
        {
            // SAFETY: Copying raw bytes from `&[u16]` on little-endian targets is zero-overhead.
            let src_bytes = unsafe {
                std::slice::from_raw_parts(depth_mm.as_ptr() as *const u8, byte_len)
            };
            payload_dst.copy_from_slice(src_bytes);
        }
        #[cfg(target_endian = "big")]
        {
            for (dst_chunk, &px) in payload_dst.chunks_exact_mut(2).zip(depth_mm.iter()) {
                dst_chunk.copy_from_slice(&px.to_le_bytes());
            }
        }

        // Publish sequence counter with Release ordering so reader observes complete header + payload.
        let hdr = ring_header_ref(&self.mmap);
        hdr.write_seq.store(next_seq, Ordering::Release);
        Ok(next_seq)
    }

    pub fn mark_stopped(&self) {
        let hdr = ring_header_ref(&self.mmap);
        hdr.producer_alive.store(0, Ordering::Release);
    }

    pub fn health(&self) -> ShmRingHealth {
        snapshot_health(ring_header_ref(&self.mmap))
    }
}

impl Drop for ShmRingWriter {
    fn drop(&mut self) {
        self.mark_stopped();
    }
}

/// Consumer handle for lock-free reading of the freshest depth frame from the ring buffer.
pub struct ShmRingReader {
    mmap: MmapMut,
    slot_count: u32,
    slot_stride: usize,
    payload_offset: usize,
    max_payload_bytes: usize,
    ring_hdr_size: usize,
    last_read_seq: u64,
}

impl ShmRingReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SidecarError> {
        let file = File::options().read(true).write(true).open(path.as_ref())?;
        let len = file.metadata()?.len() as usize;
        if len < size_of::<ShmRingHeader>() {
            return Err(SidecarError::Shm(
                "shared-memory file smaller than ring header".into(),
            ));
        }

        // SAFETY: File size checked; mapping is used for atomic read_seq updates + slot reads.
        let mmap = unsafe { MmapOptions::new().len(len).map_mut(&file)? };
        let hdr = ring_header_ref(&mmap);
        if hdr.magic != RING_MAGIC {
            return Err(SidecarError::Shm(format!(
                "invalid ring buffer magic: 0x{:08X}",
                hdr.magic
            )));
        }
        let slot_count = hdr.slot_count;
        let slot_stride = hdr.slot_stride as usize;
        let max_payload_bytes = hdr.max_payload_bytes as usize;
        let (payload_offset, expected_stride) = slot_layout(max_payload_bytes);
        if slot_count < 2 || slot_stride != expected_stride {
            return Err(SidecarError::Shm("corrupted ring buffer geometry".into()));
        }
        let ring_hdr_size = align_up(size_of::<ShmRingHeader>(), HEADER_ALIGN);
        if len < total_file_size(slot_count, slot_stride) {
            return Err(SidecarError::Shm(
                "shared-memory file shorter than declared slots".into(),
            ));
        }

        Ok(Self {
            mmap,
            slot_count,
            slot_stride,
            payload_offset,
            max_payload_bytes,
            ring_hdr_size,
            last_read_seq: 0,
        })
    }

    /// Reads the newest available 16-bit depth frame without blocking.
    /// Drops stale intermediate frames (drop-oldest freshness policy) and verifies seqlock consistency.
    pub fn try_read_latest_u16(
        &mut self,
        out_depth_mm: &mut Vec<u16>,
    ) -> Result<Option<ShmFrameHeader>, SidecarError> {
        // Retry up to 3 times if producer wraps around mid-copy.
        for _ in 0..3 {
            let hdr = ring_header_ref(&self.mmap);
            let latest_seq = hdr.write_seq.load(Ordering::Acquire);
            if latest_seq == 0 || latest_seq <= self.last_read_seq {
                return Ok(None);
            }

            let skipped = latest_seq.saturating_sub(self.last_read_seq + 1);
            let slot_idx = ((latest_seq - 1) % self.slot_count as u64) as usize;
            let slot_offset = self.ring_hdr_size + slot_idx * self.slot_stride;

            let frame_hdr: ShmFrameHeader = {
                let slice = &self.mmap[slot_offset..slot_offset + size_of::<ShmFrameHeader>()];
                // SAFETY: Reading aligned `#[repr(C)]` POD header via unaligned copy.
                unsafe { std::ptr::read_unaligned(slice.as_ptr() as *const ShmFrameHeader) }
            };

            if frame_hdr.magic != ShmFrameHeader::MAGIC || frame_hdr.frame_seq != latest_seq {
                continue;
            }

            let byte_len = frame_hdr.payload_bytes();
            if byte_len > self.max_payload_bytes || byte_len % 2 != 0 {
                return Err(SidecarError::Shm("invalid frame payload size in SHM".into()));
            }
            let pixel_count = byte_len / 2;
            out_depth_mm.resize(pixel_count, 0);

            let payload_start = slot_offset + self.payload_offset;
            let src = &self.mmap[payload_start..payload_start + byte_len];
            #[cfg(target_endian = "little")]
            {
                // SAFETY: Copying raw little-endian u16 bytes into pre-sized Vec<u16>.
                let dst = unsafe {
                    std::slice::from_raw_parts_mut(out_depth_mm.as_mut_ptr() as *mut u8, byte_len)
                };
                dst.copy_from_slice(src);
            }
            #[cfg(target_endian = "big")]
            {
                for (dst_px, chunk) in out_depth_mm.iter_mut().zip(src.chunks_exact(2)) {
                    *dst_px = u16::from_le_bytes([chunk[0], chunk[1]]);
                }
            }

            // Verify writer didn't lap this slot while we were reading it.
            let post_write_seq = hdr.write_seq.load(Ordering::Acquire);
            if post_write_seq.saturating_sub(latest_seq) >= self.slot_count as u64 {
                continue;
            }

            if skipped > 0 {
                hdr.dropped_frames.fetch_add(skipped, Ordering::Relaxed);
            }
            hdr.read_seq.store(latest_seq, Ordering::Release);
            self.last_read_seq = latest_seq;
            return Ok(Some(frame_hdr));
        }

        Ok(None)
    }

    pub fn health(&self) -> ShmRingHealth {
        snapshot_health(ring_header_ref(&self.mmap))
    }

    pub fn last_read_seq(&self) -> u64 {
        self.last_read_seq
    }
}

fn snapshot_health(hdr: &ShmRingHeader) -> ShmRingHealth {
    let write_seq = hdr.write_seq.load(Ordering::Acquire);
    let read_seq = hdr.read_seq.load(Ordering::Acquire);
    let dropped_frames = hdr.dropped_frames.load(Ordering::Relaxed);
    let backlog = write_seq.saturating_sub(read_seq) as f32;
    let utilization = (backlog / hdr.slot_count.max(1) as f32).clamp(0.0, 1.0);
    ShmRingHealth {
        slot_count: hdr.slot_count,
        write_seq,
        read_seq,
        dropped_frames,
        utilization,
        producer_pid: hdr.producer_pid.load(Ordering::Acquire),
        producer_alive: hdr.producer_alive.load(Ordering::Acquire) != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shm_ring_buffer_zero_copy_u16_roundtrip_and_drop_oldest() {
        let path = std::env::temp_dir().join(format!("kf-shm-test-{}.shm", uuid::Uuid::new_v4()));
        let mut writer = ShmRingWriter::create(&path, 4, 32, 24).unwrap();
        let mut reader = ShmRingReader::open(&path).unwrap();

        let frame1: Vec<u16> = (0..(32 * 24)).map(|i| (500 + (i % 1000)) as u16).collect();
        let seq1 = writer.write_depth_u16(32, 24, 123_456, &frame1).unwrap();
        assert_eq!(seq1, 1);

        let mut out = Vec::new();
        let hdr = reader.try_read_latest_u16(&mut out).unwrap().unwrap();
        assert_eq!(hdr.frame_seq, 1);
        assert_eq!(hdr.width, 32);
        assert_eq!(hdr.height, 24);
        assert_eq!(hdr.pixel_format(), Some(PixelFormat::Depth16));
        assert_eq!(out, frame1);

        // No new frame -> Ok(None)
        assert!(reader.try_read_latest_u16(&mut out).unwrap().is_none());

        // Write 3 frames before consumer reads -> consumer gets freshest frame (seq 4)
        for s in 2..=4u16 {
            let f = vec![s * 300; 32 * 24];
            writer.write_depth_u16(32, 24, s as u64 * 1000, &f).unwrap();
        }
        let hdr4 = reader.try_read_latest_u16(&mut out).unwrap().unwrap();
        assert_eq!(hdr4.frame_seq, 4);
        assert_eq!(out[0], 1200);
        assert!(reader.health().dropped_frames >= 2);

        let _ = std::fs::remove_file(&path);
    }
}
