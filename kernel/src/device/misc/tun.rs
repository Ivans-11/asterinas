// SPDX-License-Identifier: MPL-2.0

//! Minimal Linux-compatible TAP support.
//!
//! The implementation currently exposes one Ethernet TAP interface (`tap0`).
//! It is sufficient for VMMs such as Firecracker to exchange Ethernet frames
//! with the Asterinas network stack through `/dev/net/tun`.

use alloc::vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use aster_bigtcp::{
    device::{
        Device as NetworkDevice, DeviceCapabilities, Medium, NotifyDevice, RxToken, TxToken,
        WithDevice,
    },
    time::Instant,
};
use aster_softirq::BottomHalfDisabled;
use device_id::{DeviceId, MajorId, MinorId};
use ostd::{mm::VmIo, sync::WaitQueue};
use spin::Once;

use crate::{
    context::current_userspace,
    device::{Device, DeviceType, DevtmpfsInodeMeta, registry::char},
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, ioctl::RawIoctl},
};

const TUN_MINOR: u32 = 200;
const IFREQ_SIZE: usize = 40;
const IFNAMSIZ: usize = 16;

const IFF_TAP: u16 = 0x0002;
const IFF_NO_PI: u16 = 0x1000;
const IFF_VNET_HDR: u16 = 0x4000;
const SUPPORTED_IFF_FLAGS: u16 = IFF_TAP | IFF_NO_PI | IFF_VNET_HDR;

// _IOW('T', nr, int), as defined by Linux's <linux/if_tun.h>.
const TUNSETIFF: u32 = 0x4004_54ca;
// _IOR('T', nr, unsigned int), as defined by Linux's <linux/if_tun.h>.
const TUNGETFEATURES: u32 = 0x8004_54cf;
const TUNSETOFFLOAD: u32 = 0x4004_54d0;
const TUNSETVNETHDRSZ: u32 = 0x4004_54d8;

const DEFAULT_VNET_HDR_SIZE: usize = 12;
const SUPPORTED_VNET_HDR_SIZES: [usize; 2] = [10, 12];
const MAX_PACKET_SIZE: usize = 65_562;
const MAX_QUEUED_PACKETS: usize = 1024;
const TAP_NAME: &[u8] = b"tap0";
const TAP_MAC: [u8; 6] = [0x06, 0x00, 0xac, 0x10, 0x00, 0x01];

static TAP_STATE: Once<Arc<TapState>> = Once::new();

fn tap_state() -> &'static Arc<TapState> {
    TAP_STATE.call_once(|| Arc::new(TapState::new()))
}

struct TapState {
    /// Ethernet frames written by the VMM and waiting for the host stack.
    from_userspace: SpinLock<VecDeque<Vec<u8>>, BottomHalfDisabled>,
    /// VNET-header-prefixed frames emitted by the host stack for the VMM.
    to_userspace: SpinLock<VecDeque<Vec<u8>>, BottomHalfDisabled>,
    attached: AtomicBool,
    vnet_hdr_size: AtomicUsize,
    offload_flags: AtomicU32,
    pollee: Pollee,
    read_wait_queue: WaitQueue,
    write_wait_queue: WaitQueue,
}

impl TapState {
    fn new() -> Self {
        Self {
            from_userspace: SpinLock::new(VecDeque::new()),
            to_userspace: SpinLock::new(VecDeque::new()),
            attached: AtomicBool::new(false),
            vnet_hdr_size: AtomicUsize::new(DEFAULT_VNET_HDR_SIZE),
            offload_flags: AtomicU32::new(0),
            pollee: Pollee::new(),
            read_wait_queue: WaitQueue::new(),
            write_wait_queue: WaitQueue::new(),
        }
    }

    fn attach(&self) -> Result<()> {
        self.attached
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::with_message(Errno::EBUSY, "tap0 is already attached"))?;
        self.from_userspace.lock().clear();
        self.to_userspace.lock().clear();
        self.vnet_hdr_size
            .store(DEFAULT_VNET_HDR_SIZE, Ordering::Release);
        self.offload_flags.store(0, Ordering::Release);
        self.pollee.notify(IoEvents::OUT);
        Ok(())
    }

    fn detach(&self) {
        self.attached.store(false, Ordering::Release);
        self.from_userspace.lock().clear();
        self.to_userspace.lock().clear();
        self.pollee.notify(IoEvents::ERR | IoEvents::HUP);
        self.read_wait_queue.wake_all();
        self.write_wait_queue.wake_all();
    }

    fn is_attached(&self) -> bool {
        self.attached.load(Ordering::Acquire)
    }

    fn check_io_events(&self) -> IoEvents {
        if !self.is_attached() {
            return IoEvents::ERR | IoEvents::HUP;
        }

        let mut events = IoEvents::empty();
        if !self.to_userspace.lock().is_empty() {
            events |= IoEvents::IN;
        }
        if self.from_userspace.lock().len() < MAX_QUEUED_PACKETS {
            events |= IoEvents::OUT;
        }
        events
    }

    fn enqueue_to_userspace(&self, frame: &[u8]) -> Result<(), ()> {
        if !self.is_attached() {
            return Err(());
        }

        let header_len = self.vnet_hdr_size.load(Ordering::Acquire);
        let mut packet = vec![0u8; header_len];
        packet.extend_from_slice(frame);

        let mut queue = self.to_userspace.lock();
        if queue.len() >= MAX_QUEUED_PACKETS {
            return Err(());
        }
        queue.push_back(packet);
        drop(queue);

        self.pollee.notify(IoEvents::IN);
        self.read_wait_queue.wake_all();
        Ok(())
    }

    fn dequeue_to_userspace(&self) -> Option<Vec<u8>> {
        let packet = self.to_userspace.lock().pop_front();
        if packet.is_some() {
            self.pollee.invalidate();
        }
        packet
    }

    fn enqueue_from_userspace(&self, frame: Vec<u8>) -> Result<(), Vec<u8>> {
        if !self.is_attached() {
            return Err(frame);
        }

        let mut queue = self.from_userspace.lock();
        if queue.len() >= MAX_QUEUED_PACKETS {
            return Err(frame);
        }
        queue.push_back(frame);
        drop(queue);
        self.pollee.invalidate();
        Ok(())
    }

    fn dequeue_from_userspace(&self) -> Option<Vec<u8>> {
        let packet = self.from_userspace.lock().pop_front();
        if packet.is_some() {
            self.pollee.notify(IoEvents::OUT);
            self.write_wait_queue.wake_all();
        }
        packet
    }
}

/// A cloneable driver handle used by the host network interface.
pub(crate) struct TapDriverHandle(Arc<SpinLock<TapDriver, BottomHalfDisabled>>);

impl WithDevice for TapDriverHandle {
    type Device = TapDriver;

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Self::Device) -> R,
    {
        f(&mut self.0.lock())
    }
}

pub(crate) fn network_driver() -> TapDriverHandle {
    TapDriverHandle(Arc::new(SpinLock::new(TapDriver {
        state: tap_state().clone(),
    })))
}

pub(crate) const fn mac_addr() -> [u8; 6] {
    TAP_MAC
}

pub(crate) struct TapDriver {
    state: Arc<TapState>,
}

impl NetworkDevice for TapDriver {
    type RxToken<'a> = TapRxToken;
    type TxToken<'a> = TapTxToken;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.state.to_userspace.lock().len() >= MAX_QUEUED_PACKETS {
            return None;
        }
        let frame = self.state.dequeue_from_userspace()?;
        Some((TapRxToken(frame), TapTxToken(self.state.clone())))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        (self.state.is_attached() && self.state.to_userspace.lock().len() < MAX_QUEUED_PACKETS)
            .then(|| TapTxToken(self.state.clone()))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        // Keep the default `Checksum::Both`: this software device relies on
        // the protocol stack to verify incoming and compute outgoing checksums.
        caps
    }
}

impl NotifyDevice for TapDriver {
    fn notify_poll_end(&mut self) {}
}

pub(crate) struct TapRxToken(Vec<u8>);

impl RxToken for TapRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.0)
    }
}

pub(crate) struct TapTxToken(Arc<TapState>);

impl TxToken for TapTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut frame = vec![0u8; len];
        let result = f(&mut frame);
        let _ = self.0.enqueue_to_userspace(&frame);
        result
    }
}

#[derive(Debug)]
struct TunDevice {
    id: DeviceId,
}

impl TunDevice {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            id: DeviceId::new(MajorId::new(10), MinorId::new(TUN_MINOR)),
        })
    }
}

impl Device for TunDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new("net/tun"))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(TunFile {
            state: tap_state().clone(),
            attached: AtomicBool::new(false),
        }))
    }
}

struct TunFile {
    state: Arc<TapState>,
    attached: AtomicBool,
}

impl TunFile {
    fn ensure_attached(&self) -> Result<()> {
        if !self.attached.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EBADFD, "the tun file is not attached");
        }
        Ok(())
    }

    fn set_iff(&self, arg: usize) -> Result<i32> {
        if self.attached.load(Ordering::Acquire) {
            return_errno_with_message!(Errno::EINVAL, "the tun file is already attached");
        }

        let mut ifreq = [0u8; IFREQ_SIZE];
        current_userspace!().read_bytes(arg, &mut ifreq)?;

        let name_len = ifreq[..IFNAMSIZ]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(IFNAMSIZ);
        let name = &ifreq[..name_len];
        if !name.is_empty() && name != TAP_NAME && name != b"tap%d" {
            return_errno_with_message!(Errno::ENODEV, "only tap0 is available");
        }

        let flags = u16::from_ne_bytes([ifreq[IFNAMSIZ], ifreq[IFNAMSIZ + 1]]);
        if flags != SUPPORTED_IFF_FLAGS {
            return_errno_with_message!(
                Errno::EINVAL,
                "tap0 requires IFF_TAP | IFF_NO_PI | IFF_VNET_HDR"
            );
        }

        self.state.attach()?;
        self.attached.store(true, Ordering::Release);

        ifreq[..IFNAMSIZ].fill(0);
        ifreq[..TAP_NAME.len()].copy_from_slice(TAP_NAME);
        current_userspace!().write_bytes(arg, &ifreq)?;
        Ok(0)
    }

    fn set_vnet_hdr_size(&self, arg: usize) -> Result<i32> {
        self.ensure_attached()?;
        let mut bytes = [0u8; size_of::<i32>()];
        current_userspace!().read_bytes(arg, &mut bytes)?;
        let size = i32::from_ne_bytes(bytes);
        if size < 0 || !SUPPORTED_VNET_HDR_SIZES.contains(&(size as usize)) {
            return_errno_with_message!(Errno::EINVAL, "unsupported vnet header size");
        }
        self.state
            .vnet_hdr_size
            .store(size as usize, Ordering::Release);
        Ok(0)
    }

    fn set_offload(&self, flags: usize) -> Result<i32> {
        self.ensure_attached()?;
        self.state
            .offload_flags
            .store(flags as u32, Ordering::Release);
        Ok(0)
    }

    fn get_features(&self, arg: usize) -> Result<i32> {
        current_userspace!().write_val(arg, &(SUPPORTED_IFF_FLAGS as u32))?;
        Ok(0)
    }

    fn try_write_packet(&self, packet: &[u8]) -> Result<()> {
        let header_len = self.state.vnet_hdr_size.load(Ordering::Acquire);
        if packet.len() < header_len + 14 {
            return_errno_with_message!(Errno::EINVAL, "the TAP frame is too short");
        }

        let mut frame = packet[header_len..].to_vec();
        complete_vnet_checksum(&packet[..header_len], &mut frame)?;
        self.state
            .enqueue_from_userspace(frame)
            .map_err(|_| Error::with_message(Errno::EAGAIN, "the TAP receive queue is full"))
    }

    fn write_packet(&self, reader: &mut dyn MultiRead, status_flags: StatusFlags) -> Result<usize> {
        self.ensure_attached()?;
        let packet_len = reader.sum_lens();
        if packet_len > MAX_PACKET_SIZE {
            return_errno_with_message!(Errno::EMSGSIZE, "the TAP packet is too large");
        }

        let mut packet = vec![0u8; packet_len];
        let copied = reader.read(&mut packet.as_mut_slice().into())?;
        debug_assert_eq!(copied, packet_len);

        if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.try_write_packet(&packet)?;
        } else {
            self.state
                .write_wait_queue
                .wait_until(|| match self.try_write_packet(&packet) {
                    Ok(()) => Some(Ok(())),
                    Err(err) if err.error() == Errno::EAGAIN => None,
                    Err(err) => Some(Err(err)),
                })?;
        }

        // Deliver the frame to the host protocol stack immediately.
        crate::net::iface::tap_iface().poll();
        Ok(packet_len)
    }
}

impl Drop for TunFile {
    fn drop(&mut self) {
        if self.attached.load(Ordering::Acquire) {
            self.state.detach();
        }
    }
}

impl Pollable for TunFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.state
            .pollee
            .poll_with(mask, poller, || self.state.check_io_events())
    }
}

impl FileOps for TunFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.ensure_attached()?;
        let packet = if status_flags.contains(StatusFlags::O_NONBLOCK) {
            self.state
                .dequeue_to_userspace()
                .ok_or_else(|| Error::with_message(Errno::EAGAIN, "no TAP packet is available"))?
        } else {
            self.state
                .read_wait_queue
                .wait_until(|| self.state.dequeue_to_userspace())
        };
        let copied =
            writer.write_fallible(&mut packet[..writer.avail().min(packet.len())].into())?;
        // A read frees queue space and may allow the host stack to transmit another frame.
        crate::net::iface::tap_iface().poll();
        Ok(copied)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.write_packet(reader, status_flags)
    }
}

impl PerOpenFileOps for TunFile {
    fn write_vectored(
        &self,
        reader: &mut dyn MultiRead,
        status_flags: StatusFlags,
    ) -> Option<Result<usize>> {
        Some(self.write_packet(reader, status_flags))
    }

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "seek is not supported by /dev/net/tun")
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        match raw_ioctl.cmd() {
            TUNSETIFF => self.set_iff(raw_ioctl.arg()),
            TUNGETFEATURES => self.get_features(raw_ioctl.arg()),
            TUNSETVNETHDRSZ => self.set_vnet_hdr_size(raw_ioctl.arg()),
            TUNSETOFFLOAD => self.set_offload(raw_ioctl.arg()),
            _ => return_errno_with_message!(Errno::ENOTTY, "unsupported TUN/TAP ioctl"),
        }
    }
}

/// Completes a checksum described by a virtio-net header.
fn complete_vnet_checksum(header: &[u8], frame: &mut [u8]) -> Result<()> {
    const VIRTIO_NET_HDR_F_NEEDS_CSUM: u8 = 1;
    if header.first().copied().unwrap_or(0) & VIRTIO_NET_HDR_F_NEEDS_CSUM == 0 {
        return Ok(());
    }
    if header.len() < 10 {
        return_errno_with_message!(Errno::EINVAL, "invalid vnet header");
    }

    let checksum_start = u16::from_le_bytes([header[6], header[7]]) as usize;
    let checksum_offset = u16::from_le_bytes([header[8], header[9]]) as usize;
    let checksum_at = checksum_start
        .checked_add(checksum_offset)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid checksum offset"))?;
    if checksum_start >= frame.len() || checksum_at + 2 > frame.len() {
        return_errno_with_message!(Errno::EINVAL, "checksum offset is outside the frame");
    }

    let mut sum = 0u32;
    for bytes in frame[checksum_start..].chunks(2) {
        let word = if bytes.len() == 2 {
            u16::from_be_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_be_bytes([bytes[0], 0])
        };
        sum += word as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let mut checksum = !(sum as u16);
    if checksum == 0 {
        checksum = 0xffff;
    }
    frame[checksum_at..checksum_at + 2].copy_from_slice(&checksum.to_be_bytes());
    Ok(())
}

pub(super) fn init_in_first_kthread() {
    char::register(TunDevice::new()).unwrap();
}
