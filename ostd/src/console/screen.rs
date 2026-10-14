// SPDX-License-Identifier: MPL-2.0

//! Buffers boot output until the kernel installs its screen console.

use core::fmt::{self, Write};

use crate::sync::{LocalIrqDisabled, SpinLock};

const CAPACITY: usize = 32 * 1024;

struct ScreenOutput {
    bytes: [u8; CAPACITY],
    start: usize,
    len: usize,
    writer: Option<fn(&[u8])>,
}

static OUTPUT: SpinLock<ScreenOutput, LocalIrqDisabled> = SpinLock::new(ScreenOutput {
    bytes: [0; CAPACITY],
    start: 0,
    len: 0,
    writer: None,
});

impl Write for ScreenOutput {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if let Some(writer) = self.writer {
            writer(text.as_bytes());
        } else {
            for byte in text.bytes() {
                self.bytes[(self.start + self.len) % CAPACITY] = byte;
                if self.len == CAPACITY {
                    self.start = (self.start + 1) % CAPACITY;
                } else {
                    self.len += 1;
                }
            }
        }
        Ok(())
    }
}

/// Copies output to the screen selected by `console=tty0`.
///
/// Before registration, the most recent 32 KiB of output are retained for replay.
pub fn print(args: fmt::Arguments) {
    print_inner(args, false);
}

/// Retains logger output until the VT joins the regular console device list.
pub fn buffer(args: fmt::Arguments) {
    print_inner(args, true);
}

fn print_inner(args: fmt::Arguments, buffer_only: bool) {
    let Some(info) = crate::boot::EARLY_INFO.get() else {
        return;
    };
    if !info
        .kernel_cmdline
        .split_whitespace()
        .any(|arg| arg == "console=tty0")
    {
        return;
    }
    // Panic output must not deadlock if the console itself is already locked.
    if let Some(mut output) = OUTPUT.try_lock() {
        if buffer_only && output.writer.is_some() {
            return;
        }
        let _ = output.write_fmt(args);
    }
}

/// Installs a screen writer and replays the retained boot output once.
///
/// The writer runs with local IRQs disabled and must not sleep or log.
pub fn register(writer: fn(&[u8])) {
    let mut output = OUTPUT.lock();
    if output.writer.is_some() {
        return;
    }
    let first = output.len.min(CAPACITY - output.start);
    writer(&output.bytes[output.start..output.start + first]);
    writer(&output.bytes[..output.len - first]);
    output.len = 0;
    output.writer = Some(writer);
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn retains_latest_output_when_full() {
        let mut output = ScreenOutput {
            bytes: [0; CAPACITY],
            start: 0,
            len: 0,
            writer: None,
        };
        for _ in 0..CAPACITY {
            output.write_str("a").unwrap();
        }
        output.write_str("bc").unwrap();
        assert_eq!(output.len, CAPACITY);
        assert_eq!(output.start, 2);
        assert_eq!(output.bytes[0], b'b');
        assert_eq!(output.bytes[1], b'c');
        assert!(output.bytes[2..].iter().all(|byte| *byte == b'a'));
    }
}
