// SPDX-License-Identifier: MPL-2.0

use axvisor_api::{
    arch::CacheOp,
    memory::{PhysAddr, VirtAddr},
    time, vmm,
};

pub(crate) fn prepare_virtualization() {}

pub(crate) fn set_oneshot_timer(_deadline: time::TimeValue) {
    // Asterinas does not yet provide an aarch64 Axvisor host runtime.
}

pub(crate) fn get_host_fdt_ptr() -> Option<PhysAddr> {
    None
}

pub(crate) fn inject_virtual_interrupt(_vector: vmm::InterruptVector) {}

pub(crate) fn dcache_range(_op: CacheOp, _addr: VirtAddr, _size: usize) {}
