//! The GPU's own time to draw a frame, from timestamps written at the start
//! and the end of the render pass (the `TIMESTAMP_QUERY` feature: WebGPU and
//! native backends, not WebGL2).
//!
//! Reading them back is asynchronous: the frame's timestamps are copied to a
//! buffer mapped for the CPU, read a few frames later. One measurement is in
//! flight at a time, so frames finishing meanwhile aren't measured, and
//! nothing ever waits for the GPU.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The readback buffer: free to copy into, being mapped, or mapped.
const FREE: u8 = 0;
const MAPPING: u8 = 1;
const MAPPED: u8 = 2;

/// Two timestamps, of 8 bytes each.
const SIZE: u64 = 2 * 8;

pub struct GpuTimer {
    queries: wgpu::QuerySet,
    /// Where the queries are resolved, on the GPU.
    resolved: wgpu::Buffer,
    /// A copy the CPU can read.
    readback: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// Whether this frame's timestamps are copied to `readback`.
    copied: bool,
    /// Nanoseconds per timestamp tick.
    period: f32,
}

impl GpuTimer {
    /// `None` if the device can't write timestamps.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: SIZE,
                usage,
                mapped_at_creation: false,
            })
        };
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolved: buffer(
                "frame timestamps",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            readback: buffer(
                "frame timestamps readback",
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            ),
            state: Arc::new(AtomicU8::new(FREE)),
            copied: false,
            period: queue.get_timestamp_period(),
        })
    }

    /// For the render pass: a timestamp at its start and its end.
    pub fn timestamp_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    /// After the render pass: copies its timestamps for the CPU, unless the
    /// previous ones are still being read.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolved, 0);
        if self.state.load(Ordering::Acquire) == FREE {
            encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.readback, 0, SIZE);
            self.copied = true;
        }
    }

    /// After submitting the frame: asks for the copy to be readable.
    pub fn submitted(&mut self) {
        if !std::mem::take(&mut self.copied) {
            return;
        }
        self.state.store(MAPPING, Ordering::Release);
        let state = self.state.clone();
        self.readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FREE },
                    Ordering::Release,
                );
            });
    }

    /// The seconds the GPU took for the last frame measured, once readable.
    pub fn read(&mut self) -> Option<f32> {
        if self.state.load(Ordering::Acquire) != MAPPED {
            return None;
        }
        let ticks = self.readback.get_mapped_range(..).ok().and_then(|view| {
            let start = u64::from_le_bytes(view[0..8].try_into().unwrap());
            let end = u64::from_le_bytes(view[8..16].try_into().unwrap());
            end.checked_sub(start)
        });
        self.readback.unmap();
        self.state.store(FREE, Ordering::Release);
        // Timestamps can go backwards, e.g. when the GPU changes clocks.
        ticks.map(|ticks| ticks as f32 * self.period / 1e9)
    }
}
