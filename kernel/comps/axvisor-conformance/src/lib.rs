// SPDX-License-Identifier: MPL-2.0

//! Asterinas runner for the Axvisor host-contract conformance suite.

#![no_std]
#![deny(unsafe_code)]

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axvisor_api::{irq, task, time};
use ostd::{
    cpu::{CpuId, CpuSet},
    util::id_set::Id,
};

struct AsterinasStimulus;

impl axvisor_conformance::Stimulus for AsterinasStimulus {
    fn init_percpu(&self) {
        axvisor_core::vmm::init_timer_percpu();
    }

    fn verify_oneshot_timer(&self) -> Option<bool> {
        static FIRED: AtomicBool = AtomicBool::new(false);
        const DELAY_NANOS: u64 = 5_000_000;
        const TIMEOUT_NANOS: u64 = 500_000_000;

        FIRED.store(false, Ordering::Release);
        let deadline = time::current_time_nanos().saturating_add(DELAY_NANOS);
        axvisor_core::vmm::timer::register_timer(deadline, |_| {
            FIRED.store(true, Ordering::Release);
        });
        while !FIRED.load(Ordering::Acquire)
            && time::current_time_nanos() < deadline.saturating_add(TIMEOUT_NANOS)
        {
            task::yield_now();
        }
        Some(FIRED.load(Ordering::Acquire))
    }

    fn verify_physical_irq(&self, test_vector: usize) -> Option<bool> {
        static RESULT: AtomicUsize = AtomicUsize::new(0);
        static TEST_VECTOR: AtomicUsize = AtomicUsize::new(0);
        const TIMEOUT_NANOS: u64 = 100_000_000;

        fn forward_from_ipi() {
            let vector = TEST_VECTOR.load(Ordering::Acquire);
            RESULT.store(
                if irq::handle_irq(vector) { 1 } else { 2 },
                Ordering::Release,
            );
        }

        if ostd::cpu::num_cpus() < 2 {
            return Some(false);
        }
        RESULT.store(0, Ordering::Release);
        TEST_VECTOR.store(test_vector, Ordering::Release);
        let current = CpuId::current_racy().as_usize();
        let target = CpuId::new(if current == 0 { 1 } else { 0 });
        ostd::smp::inter_processor_call(&CpuSet::from(target), forward_from_ipi);
        let timeout = time::current_time_nanos().saturating_add(TIMEOUT_NANOS);
        while RESULT.load(Ordering::Acquire) == 0 && time::current_time_nanos() < timeout {
            task::yield_now();
        }
        Some(RESULT.load(Ordering::Acquire) == 1)
    }
}

static STIMULUS: AsterinasStimulus = AsterinasStimulus;

/// Runs the suite after the full Asterinas host runtime has been initialized.
pub fn run() {
    aster_logger::print!("[axvisor] running host-contract conformance tests\n");
    let report = axvisor_conformance::run(&STIMULUS);
    for case in report.cases() {
        aster_logger::print!(
            "CONFORMANCE family={} check={} status={}\n",
            case.family,
            case.check,
            case.outcome.as_str()
        );
    }
    aster_logger::print!(
        "CONFORMANCE summary={} complete={}\n",
        if report.passed() { "PASS" } else { "FAIL" },
        report.complete()
    );
    assert!(report.passed(), "Axvisor host-contract conformance failed");
}
