// SPDX-License-Identifier: MPL-2.0

use axvisor_api::{
    arch::CacheOp,
    memory::{PhysAddr, VirtAddr},
    time,
    types::InterruptVector,
};
use ostd::{arch::boot::DEVICE_TREE_PADDR, timer};

const NO_DEADLINE_TICKS: u64 = u64::MAX;

#[ax_percpu::def_percpu]
static TIMER_DEADLINE_TICKS: u64 = NO_DEADLINE_TICKS;

pub(crate) fn prepare_virtualization() {}

pub(crate) fn init_percpu() {
    timer::register_callback_on_cpu(|| {
        let deadline = TIMER_DEADLINE_TICKS.read_current();
        if deadline == NO_DEADLINE_TICKS {
            return;
        }

        let now_ticks = crate::host_current_ticks();
        if now_ticks < deadline {
            return;
        }

        TIMER_DEADLINE_TICKS.write_current(NO_DEADLINE_TICKS);
        axvisor_core::vmm::timer::check_events();
    });
}

pub(crate) fn set_oneshot_timer(deadline: time::TimeValue) {
    TIMER_DEADLINE_TICKS.write_current(crate::host_nanos_to_ticks(deadline.as_nanos() as u64));
}

pub(crate) fn get_host_fdt_ptr() -> Option<PhysAddr> {
    DEVICE_TREE_PADDR.get().copied().map(PhysAddr::from_usize)
}

pub(crate) fn inject_virtual_interrupt(vector: InterruptVector) {
    axvisor_core::arch::riscv64::inject_interrupt(vector as usize);
}

pub(crate) fn dcache_range(_op: CacheOp, _addr: VirtAddr, _size: usize) {}
