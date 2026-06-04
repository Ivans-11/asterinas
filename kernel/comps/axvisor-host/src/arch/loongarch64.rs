// SPDX-License-Identifier: MPL-2.0

use axvisor_api::{memory::PhysAddr, time};
use ostd::arch::boot::DEVICE_TREE_PADDR;

pub(crate) fn init_percpu() {}

pub(crate) fn set_oneshot_timer(_deadline: time::TimeValue) {
    // Asterinas does not yet expose host timer reprogramming to components.
}

pub(crate) fn host_fdt_paddr() -> Option<PhysAddr> {
    DEVICE_TREE_PADDR.get().copied().map(PhysAddr::from_usize)
}
