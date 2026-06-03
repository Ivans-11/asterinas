// SPDX-License-Identifier: MPL-2.0

use axvisor_api::memory::PhysAddr;
use ostd::mm::paddr_to_vaddr;

#[cfg_attr(target_arch = "x86_64", path = "x86_64.rs")]
#[cfg_attr(target_arch = "riscv64", path = "riscv64.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "loongarch64.rs")]
#[cfg_attr(target_arch = "aarch64", path = "aarch64.rs")]
mod imp;

pub(crate) use imp::{
    dcache_range, host_fdt_paddr, init_percpu, inject_virtual_interrupt, prepare_virtualization,
    set_oneshot_timer,
};

pub(crate) fn linear_mapping_virt_to_phys(addr: usize) -> PhysAddr {
    let linear_mapping_base = paddr_to_vaddr(0);
    PhysAddr::from_usize(
        addr.checked_sub(linear_mapping_base)
            .expect("virtual address is outside the linear-mapped physical range"),
    )
}
