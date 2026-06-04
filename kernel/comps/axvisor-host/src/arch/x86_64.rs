// SPDX-License-Identifier: MPL-2.0

use axvisor_api::time;

pub(crate) fn prepare_virtualization() {}

pub(crate) fn init_percpu() {}

pub(crate) fn set_oneshot_timer(_deadline: time::TimeValue) {
    // Asterinas does not yet expose host timer reprogramming to components.
}
