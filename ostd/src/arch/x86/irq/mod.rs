// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

// Set this module's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "irq: "
    };
}

pub(super) mod chip;
pub(super) mod ipi;
mod ops;
mod remapping;

pub use chip::{IRQ_CHIP, IrqChip, MappedIrqLine};
pub(crate) use ipi::{HwCpuId, send_ipi};
pub(crate) use ops::{
    disable_local, disable_local_and_halt, enable_local, enable_local_and_halt, is_local_enabled,
};
pub(crate) use remapping::IrqRemapping;

use crate::{
    arch::{cpu, kernel, trap::TrapFrame},
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
};

// Intel(R) 64 and IA-32 architectures Software Developer's Manual,
// Volume 3A, Section 6.2 says "Vector numbers in the range 32 to 255
// are designated as user-defined interrupts and are not reserved by
// the Intel 64 and IA-32 architecture."
pub(crate) const IRQ_NUM_MIN: u8 = 32;
pub(crate) const IRQ_NUM_MAX: u8 = 255;

/// Handles a host external interrupt reported by a hypervisor VM exit.
pub fn handle_external_interrupt(vector: usize) -> bool {
    let Ok(vector) = u8::try_from(vector) else {
        return false;
    };
    if !(IRQ_NUM_MIN..=IRQ_NUM_MAX).contains(&vector) {
        return false;
    }

    let _guard = crate::irq::disable_local();
    if ipi::vector() == Some(vector) && !crate::smp::has_pending_inter_processor_calls() {
        // The guest and host share the physical IPI vector in passthrough
        // mode.  A guest IPI has no host CALL_QUEUES entry; acknowledge it
        // here and let AxVisor forward it to the guest vIOAPIC.
        HwIrqLine::new(vector).ack();
        return true;
    }
    let trap_frame = TrapFrame {
        trap_num: vector as usize,
        ..TrapFrame::default()
    };
    call_irq_callback_functions(&trap_frame, &HwIrqLine::new(vector), PrivilegeLevel::Kernel);
    true
}

/// An IRQ line with additional information that helps acknowledge the interrupt
/// on hardware.
///
/// On x86-64, it's the hardware (i.e., the I/O APIC and local APIC) that routes
/// the interrupt to the IRQ line. Therefore, the software does not need to
/// maintain additional information about the original hardware interrupt.
pub(crate) struct HwIrqLine {
    irq_num: u8,
}

impl HwIrqLine {
    pub(super) fn new(irq_num: u8) -> Self {
        Self { irq_num }
    }

    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        debug_assert!(!cpu::context::CpuException::is_cpu_exception(
            self.irq_num as usize
        ));
        // TODO: We're in the interrupt context, so `disable_preempt()` is not
        // really necessary here.
        kernel::apic::get_or_init(&crate::task::disable_preempt() as _).eoi();
    }
}
