// SPDX-License-Identifier: MPL-2.0

//! Utilities shared by Asterinas drivers when AxVisor owns host resources.

/// Returns whether an MMIO range is reserved for an AxVisor static guest.
///
/// Axvisor test tooling passes one `axvisor.passthrough_mmio=<base>:<size>`
/// kernel argument per passthrough MMIO range from the VM configuration. Host
/// drivers should skip probing ranges that overlap these guest-owned regions.
pub fn is_static_passthrough_mmio_range(start: usize, size: usize) -> bool {
    if !is_axvisor_static_mode() {
        return false;
    }

    let Some(end) = start.checked_add(size) else {
        return false;
    };
    ostd::boot::boot_info()
        .kernel_cmdline
        .split_whitespace()
        .filter_map(|arg| arg.strip_prefix("axvisor.passthrough_mmio="))
        .filter_map(parse_mmio_range)
        .any(|(passthrough_start, passthrough_size)| {
            let Some(passthrough_end) = passthrough_start.checked_add(passthrough_size) else {
                return false;
            };
            start < passthrough_end && passthrough_start < end
        })
}

fn is_axvisor_static_mode() -> bool {
    ostd::boot::boot_info()
        .kernel_cmdline
        .split_whitespace()
        .any(|arg| arg == "axvisor.mode=static")
}

fn parse_mmio_range(value: &str) -> Option<(usize, usize)> {
    let (start, size) = value.split_once(':')?;
    Some((parse_usize(start)?, parse_usize(size)?))
}

fn parse_usize(value: &str) -> Option<usize> {
    let (digits, radix) = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        (hex, 16usize)
    } else {
        (value, 10usize)
    };

    let mut result = 0usize;
    let mut seen_digit = false;
    for byte in digits.bytes() {
        if byte == b'_' {
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => usize::from(byte - b'0'),
            b'a'..=b'f' if radix == 16 => usize::from(byte - b'a') + 10,
            b'A'..=b'F' if radix == 16 => usize::from(byte - b'A') + 10,
            _ => return None,
        };
        if digit >= radix {
            return None;
        }
        result = result.checked_mul(radix)?.checked_add(digit)?;
        seen_digit = true;
    }

    seen_digit.then_some(result)
}
