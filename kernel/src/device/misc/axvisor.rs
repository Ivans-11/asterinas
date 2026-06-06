// SPDX-License-Identifier: MPL-2.0

//! KVM-compatible AxVisor misc-device support.

use aster_axvisor_host::{
    AxError, AxErrorKind, AxResult, ControlEndpointRuntime,
    control::{ControlOps, EndpointId, EndpointSpec, SessionId},
};
use device_id::{DeviceId, MinorId};

use crate::{
    device::{Device, DeviceType, DevtmpfsInodeMeta, registry::char},
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

const KVM_MINOR: u32 = 232;

static KVM_ENDPOINT: Mutex<Option<EndpointEntry>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct EndpointEntry {
    id: EndpointId,
    ops: ControlOps,
}

pub(super) struct AxvisorControlEndpointRuntime;

impl ControlEndpointRuntime for AxvisorControlEndpointRuntime {
    fn register_endpoint(&self, spec: EndpointSpec) -> AxResult<EndpointId> {
        if spec.name != "kvm" {
            return Err(AxErrorKind::InvalidInput.into());
        }

        let mut endpoint = KVM_ENDPOINT.lock();
        if endpoint.is_some() {
            return Err(AxErrorKind::AlreadyExists.into());
        }

        let id = 1;
        *endpoint = Some(EndpointEntry { id, ops: spec.ops });
        char::register(KvmDevice::new()).map_err(to_ax_error)?;

        Ok(id)
    }

    fn unregister_endpoint(&self, id: EndpointId) -> AxResult {
        let mut endpoint = KVM_ENDPOINT.lock();
        match *endpoint {
            Some(entry) if entry.id == id => {
                *endpoint = None;
                Ok(())
            }
            _ => Err(AxErrorKind::NotFound.into()),
        }
    }
}

/// The `/dev/kvm` device.
#[derive(Debug)]
struct KvmDevice {
    id: DeviceId,
}

impl KvmDevice {
    fn new() -> Arc<Self> {
        let major = super::MISC_MAJOR.get().unwrap().get();
        let minor = MinorId::new(KVM_MINOR);

        let id = DeviceId::new(major, minor);
        Arc::new(Self { id })
    }
}

impl Device for KvmDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.id
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new("kvm"))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let ops = endpoint_ops()?;
        let session = (ops.open)().map_err(from_ax_error)?;
        Ok(Box::new(KvmFile { session, ops }))
    }
}

struct KvmFile {
    session: SessionId,
    ops: ControlOps,
}

impl Drop for KvmFile {
    fn drop(&mut self) {
        let _ = (self.ops.release)(self.session);
    }
}

impl Pollable for KvmFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        if let Some(poll) = self.ops.poll {
            match poll(self.session) {
                Ok(events) => {
                    let mut ready = IoEvents::empty();
                    if events.readable {
                        ready |= IoEvents::IN;
                    }
                    if events.writable {
                        ready |= IoEvents::OUT;
                    }
                    if events.error {
                        ready |= IoEvents::ERR;
                    }
                    ready & mask
                }
                Err(_) => IoEvents::ERR & mask,
            }
        } else {
            (IoEvents::IN | IoEvents::OUT) & mask
        }
    }
}

impl FileOps for KvmFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let Some(read) = self.ops.read else {
            return_errno_with_message!(Errno::EINVAL, "read is not supported by /dev/kvm");
        };

        let mut buf = vec![0; writer.avail()];
        let count = read(self.session, &mut buf).map_err(from_ax_error)?;
        writer.write_fallible(&mut VmReader::from(buf[..count].as_ref()))?;
        Ok(count)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let Some(write) = self.ops.write else {
            return_errno_with_message!(Errno::EINVAL, "write is not supported by /dev/kvm");
        };

        let mut buf = vec![0; reader.remain()];
        reader.read_fallible(&mut VmWriter::from(buf.as_mut_slice()))?;
        write(self.session, &buf).map_err(from_ax_error)
    }
}

impl PerOpenFileOps for KvmFile {
    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "seek is not supported")
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn mappable(&self) -> Result<crate::fs::file::Mappable> {
        if self.ops.mmap.is_none() {
            return_errno_with_message!(Errno::ENODEV, "mmap is not supported by /dev/kvm");
        }
        return_errno_with_message!(Errno::ENODEV, "mmap dispatch is not implemented yet")
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        let result = (self.ops.ioctl)(self.session, raw_ioctl.cmd(), raw_ioctl.arg())
            .map_err(from_ax_error)?;
        i32::try_from(result)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "ioctl return value overflow"))
    }
}

fn endpoint_ops() -> Result<ControlOps> {
    KVM_ENDPOINT
        .lock()
        .map(|entry| entry.ops)
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "the KVM endpoint is not registered"))
}

fn to_ax_error(err: Error) -> AxError {
    match err.error() {
        Errno::EEXIST => AxErrorKind::AlreadyExists.into(),
        Errno::EINVAL => AxErrorKind::InvalidInput.into(),
        Errno::ENOMEM => AxErrorKind::NoMemory.into(),
        _ => AxErrorKind::Io.into(),
    }
}

fn from_ax_error(err: AxError) -> Error {
    let kind = AxErrorKind::try_from(err).unwrap_or(AxErrorKind::Io);
    let errno = match kind {
        AxErrorKind::AlreadyExists => Errno::EEXIST,
        AxErrorKind::InvalidInput => Errno::EINVAL,
        AxErrorKind::NotFound => Errno::ENOENT,
        AxErrorKind::Unsupported => Errno::ENOTTY,
        AxErrorKind::PermissionDenied => Errno::EACCES,
        AxErrorKind::WouldBlock => Errno::EAGAIN,
        AxErrorKind::Interrupted => Errno::EINTR,
        AxErrorKind::NoMemory => Errno::ENOMEM,
        AxErrorKind::ResourceBusy => Errno::EBUSY,
        _ => Errno::EIO,
    };
    Error::with_message(errno, "AxVisor control operation failed")
}
