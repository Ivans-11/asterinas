// SPDX-License-Identifier: MPL-2.0

use axvisor_api::{memory::PhysAddr, time};

pub(crate) fn init_percpu() {}

pub(crate) fn set_oneshot_timer(_deadline: time::TimeValue) {
    // Asterinas does not yet provide an aarch64 Axvisor host runtime.
}

pub(crate) fn host_fdt_paddr() -> Option<PhysAddr> {
    None
}
