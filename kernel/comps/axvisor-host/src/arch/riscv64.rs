// SPDX-License-Identifier: MPL-2.0

use axvisor_api::{
    arch::CacheOp,
    memory::{PhysAddr, VirtAddr},
    time,
    types::InterruptVector,
};
use ostd::{arch::boot::DEVICE_TREE_PADDR, timer};

const NO_DEADLINE_TICKS: u64 = u64::MAX;
const NANOS_PER_SEC: u128 = 1_000_000_000;

#[ax_percpu::def_percpu]
static TIMER_DEADLINE_TICKS: u64 = NO_DEADLINE_TICKS;

pub(crate) fn prepare_virtualization() {}

pub(crate) fn init_percpu() {
    timer::register_callback_on_cpu(|| {
        let deadline = TIMER_DEADLINE_TICKS.read_current();
        if deadline == NO_DEADLINE_TICKS {
            return;
        }

        let now_ticks = ostd::arch::read_tsc();
        if now_ticks < deadline {
            return;
        }

        TIMER_DEADLINE_TICKS.write_current(NO_DEADLINE_TICKS);
        axvisor_core::vmm::timer::check_events();
    });
}

pub(crate) fn set_oneshot_timer(deadline: time::TimeValue) {
    TIMER_DEADLINE_TICKS.write_current(nanos_to_ticks(deadline.as_nanos() as u64));
}

fn nanos_to_ticks(nanos: u64) -> u64 {
    let freq = ostd::arch::tsc_freq() as u128;
    (((nanos as u128) * freq) / NANOS_PER_SEC).min(u64::MAX as u128) as u64
}

pub(crate) fn host_fdt_paddr() -> Option<PhysAddr> {
    DEVICE_TREE_PADDR.get().copied().map(PhysAddr::from_usize)
}

pub(crate) fn inject_virtual_interrupt(vector: InterruptVector) {
    axvisor_core::arch::riscv64::inject_interrupt(vector as usize);
}

pub(crate) fn dcache_range(_op: CacheOp, _addr: VirtAddr, _size: usize) {}
