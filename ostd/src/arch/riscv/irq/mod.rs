// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

pub(super) mod chip;
pub(super) mod ipi;
mod ops;
mod remapping;

pub use chip::{IRQ_CHIP, InterruptSourceInFdt, IrqChip, MappedIrqLine};
pub(crate) use ipi::{HwCpuId, send_ipi};
pub(crate) use ops::{
    disable_local, disable_local_and_halt, enable_local, enable_local_and_halt, is_local_enabled,
};
pub(crate) use remapping::IrqRemapping;

use crate::{
    arch::{cpu::context::GeneralRegs, irq::chip::InterruptSourceOnChip, trap::TrapFrame},
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
};

pub(crate) const IRQ_NUM_MIN: u8 = 0;
pub(crate) const IRQ_NUM_MAX: u8 = 255;

/// An IRQ line with additional information that helps acknowledge the interrupt
/// on hardware.
///
/// On RISC-V, it's the software that routes the interrupt to the IRQ line.
/// Therefore, the software needs to maintain interrupt source information that
/// bridges between software abstraction (e.g., `IRQ_CHIP`) and hardware
/// mechanism (e.g., PLIC).
pub(crate) struct HwIrqLine {
    irq_num: u8,
    source: InterruptSource,
}

pub(super) enum InterruptSource {
    Timer,
    #[expect(private_interfaces)]
    External(InterruptSourceOnChip),
    Software,
}

/// Claims each currently pending supervisor external interrupt and passes its
/// hardware IRQ number to `handler`.
///
/// This helper is intended for hypervisor VM-exit handling paths that want to
/// drain host pending external IRQs in task context.
pub fn for_each_pending_external_interrupt<F: FnMut(usize)>(mut handler: F) {
    let _guard = crate::irq::disable_local();
    let hart_id = crate::arch::boot::smp::get_current_hart_id();

    while let Some(hw_irq_line) = IRQ_CHIP.get().unwrap().claim_interrupt(hart_id) {
        let InterruptSource::External(source) = hw_irq_line.source else {
            continue;
        };
        handler(source.interrupt() as usize);
    }
}

/// Handles pending supervisor external interrupts with the host IRQ callbacks.
pub fn handle_pending_external_interrupts() {
    let _guard = crate::irq::disable_local();
    let hart_id = crate::arch::boot::smp::get_current_hart_id();
    let trap_frame = TrapFrame {
        general: GeneralRegs::default(),
        sstatus: 0,
        sepc: 0,
    };

    while let Some(hw_irq_line) = IRQ_CHIP.get().unwrap().claim_interrupt(hart_id) {
        call_irq_callback_functions(&trap_frame, &hw_irq_line, PrivilegeLevel::Kernel);
    }
}

impl HwIrqLine {
    pub(super) fn new(irq_num: u8, source: InterruptSource) -> Self {
        Self { irq_num, source }
    }

    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        match &self.source {
            InterruptSource::Timer => {}
            InterruptSource::External(interrupt_source_on_chip) => {
                IRQ_CHIP.get().unwrap().complete_interrupt(
                    // No races because we are in IRQs.
                    crate::arch::boot::smp::get_current_hart_id(),
                    *interrupt_source_on_chip,
                );
            }
            InterruptSource::Software => {
                // SAFETY: We have already handled the IPI. So clearing the
                // software interrupt pending bit is safe.
                unsafe { riscv::register::sip::clear_ssoft() };
            }
        }
    }
}
