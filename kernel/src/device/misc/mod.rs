// SPDX-License-Identifier: MPL-2.0

//! Misc devices.
//!
//! Character device with major number 10.

use device_id::MajorId;
use spin::Once;

use super::registry::char::{MajorIdOwner, acquire_major};

#[cfg(feature = "axvisor")]
mod axvisor;
mod hwrng;
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub mod tdxguest;
pub(crate) mod tun;

static MISC_MAJOR: Once<MajorIdOwner> = Once::new();

pub(super) fn init_in_first_kthread() {
    MISC_MAJOR.call_once(|| acquire_major(MajorId::new(10)).unwrap());

    hwrng::init_in_first_kthread();
    tun::init_in_first_kthread();

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        tdxguest::init().unwrap();
    });
}

#[cfg(feature = "axvisor")]
pub(crate) fn axvisor_control_endpoint_runtime()
-> &'static dyn aster_axvisor_host::ControlEndpointRuntime {
    &axvisor::AxvisorControlEndpointRuntime
}
