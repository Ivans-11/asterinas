// SPDX-License-Identifier: MPL-2.0

use axvisor_api::{
    arch::CacheOp,
    memory::{PhysAddr, VirtAddr},
    time,
    types::InterruptVector,
};

pub(crate) fn prepare_virtualization() {}

pub(crate) fn init_percpu() {}

pub(crate) fn set_oneshot_timer(_deadline: time::TimeValue) {
    // Asterinas does not yet expose host timer reprogramming to components.
}

pub(crate) fn host_fdt_paddr() -> Option<PhysAddr> {
    None
}

pub(crate) fn inject_virtual_interrupt(vector: InterruptVector) {
    axvisor_core::arch::x86_64::inject_interrupt(vector);
}

pub(crate) fn dcache_range(_op: CacheOp, _addr: VirtAddr, _size: usize) {}
