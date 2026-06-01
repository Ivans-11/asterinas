// SPDX-License-Identifier: MPL-2.0

use axvisor_api::{
    arch::CacheOp,
    memory::{PhysAddr, VirtAddr},
    time, vmm,
};
use ostd::arch::boot::DEVICE_TREE;

pub(crate) fn prepare_virtualization() {}

pub(crate) fn set_oneshot_timer(_deadline: time::TimeValue) {
    // Asterinas does not yet expose host timer reprogramming to components.
}

pub(crate) fn get_host_fdt_ptr() -> Option<PhysAddr> {
    DEVICE_TREE
        .get()
        .map(|device_tree| super::linear_mapping_slice_to_phys(device_tree.as_slice()))
}

pub(crate) fn inject_virtual_interrupt(vector: vmm::InterruptVector) {
    axvisor_core::arch::loongarch64::inject_interrupt(vector as usize);
}

pub(crate) fn dcache_range(_op: CacheOp, _addr: VirtAddr, _size: usize) {}
