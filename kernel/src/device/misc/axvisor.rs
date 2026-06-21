// SPDX-License-Identifier: MPL-2.0

//! KVM-compatible AxVisor misc-device support.

use core::{
    fmt::Display,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use aster_axvisor_host::{
    AxError, AxErrorKind, AxResult, ControlEndpointRuntime,
    control::{self, ControlOps, EndpointId, EndpointSpec, HostFd, SessionId},
};
use device_id::{DeviceId, MinorId};
use ostd::{
    mm::{HasPaddr, UFrame, VmIo},
    task::Task,
};

use crate::{
    device::{Device, DeviceType, DevtmpfsInodeMeta, registry::char},
    events::IoEvents,
    fs::{
        file::{
            AccessMode, CreationFlags, FileLike, Mappable, PerOpenFileOps, StatusFlags,
            file_table::{FdFlags, FileDesc},
        },
        pseudofs::AnonInodeFs,
        vfs::{
            inode::{FileOps, Inode},
            path::Path,
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
    vm::page_cache::{Vmo, VmoOptions},
};

const KVM_MINOR: u32 = 232;

static KVM_ENDPOINT: Mutex<Option<EndpointEntry>> = Mutex::new(None);
static NEXT_ACQUIRED_USER_MEMORY: AtomicU64 = AtomicU64::new(1);
static NEXT_USER_MAPPING: AtomicU64 = AtomicU64::new(1);
static NEXT_USER_NOTIFIER: AtomicU64 = AtomicU64::new(1);
static ACQUIRED_USER_MEMORY: Mutex<BTreeMap<control::UserMemoryHandle, Vec<UFrame>>> =
    Mutex::new(BTreeMap::new());
// Keep non-owning references here so closing the userspace fd still drops the
// backing object and releases the AxVisor session.
static USER_MAPPINGS: Mutex<BTreeMap<control::UserMappingHandle, Weak<KvmAnonFile>>> =
    Mutex::new(BTreeMap::new());
static USER_NOTIFIERS: Mutex<BTreeMap<control::UserNotifierHandle, Arc<dyn FileLike>>> =
    Mutex::new(BTreeMap::new());

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

    fn create_user_handle(
        &self,
        endpoint: EndpointId,
        session: SessionId,
        shared_mapping_size: usize,
    ) -> AxResult<control::CreatedUserHandle> {
        let ops = {
            let registered = KVM_ENDPOINT.lock();
            match *registered {
                Some(entry) if entry.id == endpoint => entry.ops,
                _ => return Err(AxErrorKind::NotFound.into()),
            }
        };

        create_user_handle(session, ops, shared_mapping_size).map_err(to_ax_error)
    }

    fn write_user_mapping(
        &self,
        handle: control::UserMappingHandle,
        offset: usize,
        buf: &[u8],
    ) -> AxResult {
        write_user_mapping(handle, offset, buf).map_err(to_ax_error)
    }

    fn read_user_mapping(
        &self,
        handle: control::UserMappingHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> AxResult {
        read_user_mapping(handle, offset, buf).map_err(to_ax_error)
    }

    fn release_user_mapping(&self, handle: control::UserMappingHandle) -> AxResult {
        release_user_mapping(handle).map_err(to_ax_error)
    }

    fn read_user(&self, addr: usize, buf: &mut [u8]) -> AxResult {
        read_user(addr, buf).map_err(to_ax_error)
    }

    fn write_user(&self, addr: usize, buf: &[u8]) -> AxResult {
        write_user(addr, buf).map_err(to_ax_error)
    }

    fn acquire_user_notifier(&self, fd: HostFd) -> AxResult<control::UserNotifierHandle> {
        acquire_user_notifier(fd).map_err(to_ax_error)
    }

    fn signal_user_notifier(&self, handle: control::UserNotifierHandle) -> AxResult {
        signal_user_notifier(handle).map_err(to_ax_error)
    }

    fn release_user_notifier(&self, handle: control::UserNotifierHandle) -> AxResult {
        release_user_notifier(handle).map_err(to_ax_error)
    }

    fn acquire_user_memory(
        &self,
        addr: usize,
        len: usize,
        writable: bool,
    ) -> AxResult<control::AcquiredUserMemory> {
        acquire_user_memory(addr, len, writable).map_err(to_ax_error)
    }

    fn release_user_memory(&self, handle: control::UserMemoryHandle) -> AxResult {
        release_user_memory(handle).map_err(to_ax_error)
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

fn axvisor_user_handle_name(_: &dyn Inode) -> String {
    "anon_inode:[axvisor-user-handle]".to_string()
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

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        let result = (self.ops.ioctl)(self.session, raw_ioctl.cmd(), raw_ioctl.arg())
            .map_err(from_ax_error)?;
        i32::try_from(result)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "ioctl return value overflow"))
    }
}

struct KvmAnonFile {
    session: SessionId,
    ops: ControlOps,
    owns_session: AtomicBool,
    shared_mapping: Option<Arc<Vmo>>,
    pseudo_path: Path,
}

impl KvmAnonFile {
    fn new(session: SessionId, ops: ControlOps, shared_mapping: Option<Arc<Vmo>>) -> Self {
        Self {
            session,
            ops,
            owns_session: AtomicBool::new(false),
            shared_mapping,
            pseudo_path: AnonInodeFs::new_path(axvisor_user_handle_name),
        }
    }

    fn adopt_session(&self) {
        self.owns_session.store(true, Ordering::Release);
    }

    fn shared_mapping(&self) -> Result<Arc<Vmo>> {
        self.shared_mapping
            .clone()
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "mmap is not supported"))
    }
}

impl Drop for KvmAnonFile {
    fn drop(&mut self) {
        if self.owns_session.load(Ordering::Acquire) {
            let _ = (self.ops.release)(self.session);
        }
    }
}

impl Pollable for KvmAnonFile {
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

impl FileLike for KvmAnonFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        let result = (self.ops.ioctl)(self.session, raw_ioctl.cmd(), raw_ioctl.arg())
            .map_err(from_ax_error)?;
        i32::try_from(result)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "ioctl return value overflow"))
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn mappable(&self) -> Result<Mappable> {
        self.shared_mapping
            .clone()
            .map(Mappable::Vmo)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "mmap is not supported"))
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<KvmAnonFile>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }

        Box::new(FdInfo {
            inner: self,
            fd_flags,
        })
    }
}

fn create_user_handle(
    session: SessionId,
    ops: ControlOps,
    shared_mapping_size: usize,
) -> Result<control::CreatedUserHandle> {
    let (mapping_handle, shared_mapping) = if shared_mapping_size == 0 {
        (None, None)
    } else {
        let handle = NEXT_USER_MAPPING.fetch_add(1, Ordering::Relaxed);
        if handle == 0 {
            return_errno_with_message!(Errno::EOVERFLOW, "user mapping handle overflow");
        }
        (Some(handle), Some(VmoOptions::new(shared_mapping_size).alloc()?))
    };
    let anon_file = Arc::new(KvmAnonFile::new(session, ops, shared_mapping.clone()));
    if let Some(handle) = mapping_handle {
        USER_MAPPINGS
            .lock()
            .insert(handle, Arc::downgrade(&anon_file));
    }
    match create_object_fd(anon_file.clone()) {
        Ok(fd) => {
            anon_file.adopt_session();
            Ok(control::CreatedUserHandle {
                fd,
                mapping: mapping_handle,
            })
        }
        Err(err) => {
            if let Some(handle) = mapping_handle {
                USER_MAPPINGS.lock().remove(&handle);
            }
            Err(err)
        }
    }
}

fn create_object_fd(file: Arc<dyn FileLike>) -> Result<HostFd> {
    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    let file_table = thread_local.borrow_file_table();

    let fd = file_table.unwrap().write().insert(file, FdFlags::empty());
    Ok(fd.into())
}

fn read_user(addr: usize, buf: &mut [u8]) -> Result<()> {
    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    CurrentUserSpace::new(thread_local).read_bytes(addr, buf)?;
    Ok(())
}

fn write_user(addr: usize, buf: &[u8]) -> Result<()> {
    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    CurrentUserSpace::new(thread_local).write_bytes(addr, buf)?;
    Ok(())
}

fn acquire_user_notifier(fd: HostFd) -> Result<control::UserNotifierHandle> {
    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    let file_table = thread_local.borrow_file_table();
    let file = file_table
        .unwrap()
        .read()
        .get_file(FileDesc::try_from(fd)?)
        .cloned()?;
    let handle = NEXT_USER_NOTIFIER.fetch_add(1, Ordering::Relaxed);
    if handle == 0 {
        return_errno_with_message!(Errno::EOVERFLOW, "user notifier handle overflow");
    }
    USER_NOTIFIERS.lock().insert(handle, file);
    Ok(handle)
}

fn signal_user_notifier(handle: control::UserNotifierHandle) -> Result<()> {
    let file = USER_NOTIFIERS
        .lock()
        .get(&handle)
        .cloned()
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user notifier not found"))?;

    let value = 1u64.to_ne_bytes();
    let mut reader = VmReader::from(value.as_slice()).to_fallible();
    let written = file.write(&mut reader)?;
    if written != value.len() {
        return_errno_with_message!(Errno::EIO, "short eventfd write");
    }
    Ok(())
}

fn release_user_notifier(handle: control::UserNotifierHandle) -> Result<()> {
    USER_NOTIFIERS
        .lock()
        .remove(&handle)
        .map(|_| ())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user notifier not found"))
}

fn shared_mapping_for_handle(handle: control::UserMappingHandle) -> Result<Arc<Vmo>> {
    let user_handle = USER_MAPPINGS
        .lock()
        .get(&handle)
        .cloned()
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user handle mapping not found"))?;
    let user_handle = user_handle
        .upgrade()
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user handle mapping not found"))?;
    user_handle.shared_mapping()
}

fn write_user_mapping(handle: control::UserMappingHandle, offset: usize, buf: &[u8]) -> Result<()> {
    let shared_mapping = shared_mapping_for_handle(handle)?;
    shared_mapping.write(offset, &mut VmReader::from(buf).to_fallible())?;
    Ok(())
}

fn read_user_mapping(
    handle: control::UserMappingHandle,
    offset: usize,
    buf: &mut [u8],
) -> Result<()> {
    let shared_mapping = shared_mapping_for_handle(handle)?;
    shared_mapping.read(offset, &mut VmWriter::from(buf).to_fallible())?;
    Ok(())
}

fn release_user_mapping(handle: control::UserMappingHandle) -> Result<()> {
    USER_MAPPINGS
        .lock()
        .remove(&handle)
        .map(|_| ())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user handle mapping not found"))
}

fn acquire_user_memory(
    addr: usize,
    len: usize,
    writable: bool,
) -> Result<control::AcquiredUserMemory> {
    if !addr.is_multiple_of(PAGE_SIZE) || !len.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "acquired user memory must be page aligned");
    }

    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    let user_space = CurrentUserSpace::new(thread_local);
    let frames = user_space.vmar().acquire_pages_alien(addr, len, writable)?;
    let pages = frames.iter().map(|frame| frame.paddr().into()).collect();

    let handle = NEXT_ACQUIRED_USER_MEMORY.fetch_add(1, Ordering::Relaxed);
    if handle == 0 {
        return_errno_with_message!(Errno::EOVERFLOW, "acquired user memory handle overflow");
    }
    ACQUIRED_USER_MEMORY.lock().insert(handle, frames);

    Ok(control::AcquiredUserMemory { handle, pages })
}

fn release_user_memory(handle: control::UserMemoryHandle) -> Result<()> {
    ACQUIRED_USER_MEMORY
        .lock()
        .remove(&handle)
        .map(|_| ())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "acquired user memory not found"))
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
        Errno::EFAULT => AxErrorKind::BadAddress.into(),
        Errno::EINVAL => AxErrorKind::InvalidInput.into(),
        Errno::ENOMEM => AxErrorKind::NoMemory.into(),
        _ => AxErrorKind::Io.into(),
    }
}

fn from_ax_error(err: AxError) -> Error {
    let kind = AxErrorKind::try_from(err).unwrap_or(AxErrorKind::Io);
    let errno = match kind {
        AxErrorKind::AlreadyExists => Errno::EEXIST,
        AxErrorKind::BadAddress => Errno::EFAULT,
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
