// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{FrameAllocOptions, PAGE_SIZE, Segment, VmIo},
    prelude::*,
};

pub struct Queue {
    segment: Segment<()>,
    queue_size: usize,
    tail: usize,
}

impl Queue {
    pub fn append_descriptor(&mut self, descriptor: u128) {
        self.segment
            .write_val(self.tail * size_of::<u128>(), &descriptor)
            .unwrap();
        self.tail += 1;
        // The hardware tail must wrap immediately, including when a submitted
        // batch ends exactly at the last slot of the ring.
        if self.tail == self.queue_size {
            self.tail = 0;
        }
    }

    pub fn tail(&self) -> usize {
        self.tail
    }

    pub fn size(&self) -> usize {
        self.queue_size
    }

    pub(crate) fn base_paddr(&self) -> Paddr {
        self.segment.paddr()
    }

    pub(super) fn new() -> Self {
        const DEFAULT_PAGES: usize = 1;
        let segment = FrameAllocOptions::new()
            .alloc_segment(DEFAULT_PAGES)
            .unwrap();
        Self {
            segment,
            queue_size: (DEFAULT_PAGES * PAGE_SIZE) / size_of::<u128>(),
            tail: 0,
        }
    }
}

#[cfg(ktest)]
mod test {
    use super::*;

    #[ktest]
    fn tail_wraps_at_submission_boundary() {
        let mut queue = Queue::new();
        for index in 0..queue.size() {
            queue.append_descriptor(index as u128);
            assert_eq!(queue.tail(), (index + 1) % queue.size());
        }
        queue.append_descriptor(0x1234);
        assert_eq!(queue.tail(), 1);
        assert_eq!(queue.segment.read_val::<u128>(0).unwrap(), 0x1234);
        assert_eq!(queue.segment.read_val::<u128>(16).unwrap(), 1);
    }
}
