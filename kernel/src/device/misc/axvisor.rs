// SPDX-License-Identifier: MPL-2.0

//! KVM-compatible AxVisor misc-device support.

use core::{
    fmt::Display,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use aster_axvisor_host::{
    AxError, AxErrorKind, AxResult, ControlEndpointRuntime,
    control::{self, ControlFileId, ControlOps, Fd},
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
    process::signal::{PollHandle, Pollable, Poller},
    util::ioctl::RawIoctl,
    vm::page_cache::{Vmo, VmoOptions},
};

const KVM_MINOR: u32 = 232;

static KVM_ENDPOINT: Mutex<Option<ControlOps>> = Mutex::new(None);
static NEXT_MMAP_AREA: AtomicU64 = AtomicU64::new(1);
static MMAP_AREAS: Mutex<BTreeMap<control::MmapAreaId, Arc<Vmo>>> = Mutex::new(BTreeMap::new());
static NEXT_USER_FD: AtomicU64 = AtomicU64::new(1);
static USER_FD_REFS: Mutex<BTreeMap<control::UserFdRefId, Arc<dyn FileLike>>> =
    Mutex::new(BTreeMap::new());
static NEXT_PINNED_USER_PAGES: AtomicU64 = AtomicU64::new(1);
static PINNED_USER_PAGES: Mutex<BTreeMap<control::PinnedUserPagesId, Vec<UFrame>>> =
    Mutex::new(BTreeMap::new());

pub(super) struct AxvisorControlEndpointRuntime;

impl ControlEndpointRuntime for AxvisorControlEndpointRuntime {
    fn register_endpoint(&self, ops: ControlOps) -> AxResult {
        let mut endpoint = KVM_ENDPOINT.lock();
        if endpoint.is_some() {
            return Err(AxErrorKind::AlreadyExists.into());
        }

        *endpoint = Some(ops);
        if let Err(err) = char::register(KvmDevice::new()).map_err(to_ax_error) {
            *endpoint = None;
            return Err(err);
        }

        Ok(())
    }

    fn create_user_fd(
        &self,
        control_file: ControlFileId,
        ops: ControlOps,
        mmap_area: Option<control::MmapAreaId>,
    ) -> AxResult<Fd> {
        create_user_fd(control_file, ops, mmap_area).map_err(to_ax_error)
    }

    fn get_user_fd_ref(&self, fd: Fd) -> AxResult<control::UserFdRefId> {
        get_user_fd_ref(fd).map_err(to_ax_error)
    }

    fn write_user_fd_ref(&self, user_fd_ref: control::UserFdRefId, buf: &[u8]) -> AxResult<usize> {
        write_user_fd_ref(user_fd_ref, buf).map_err(to_ax_error)
    }

    fn read_user_fd_ref(
        &self,
        user_fd_ref: control::UserFdRefId,
        buf: &mut [u8],
    ) -> AxResult<usize> {
        read_user_fd_ref(user_fd_ref, buf).map_err(to_ax_error)
    }

    fn release_user_fd_ref(&self, user_fd_ref: control::UserFdRefId) -> AxResult {
        release_user_fd_ref(user_fd_ref).map_err(to_ax_error)
    }

    fn create_mmap_area(&self, len: usize) -> AxResult<control::MmapAreaId> {
        create_mmap_area(len).map_err(to_ax_error)
    }

    fn read_mmap_area(&self, area: control::MmapAreaId, offset: usize, buf: &mut [u8]) -> AxResult {
        read_mmap_area(area, offset, buf).map_err(to_ax_error)
    }

    fn write_mmap_area(&self, area: control::MmapAreaId, offset: usize, buf: &[u8]) -> AxResult {
        write_mmap_area(area, offset, buf).map_err(to_ax_error)
    }

    fn release_mmap_area(&self, area: control::MmapAreaId) -> AxResult {
        release_mmap_area(area).map_err(to_ax_error)
    }

    fn copy_from_user(&self, addr: usize, buf: &mut [u8]) -> AxResult {
        copy_from_user(addr, buf).map_err(to_ax_error)
    }

    fn copy_to_user(&self, addr: usize, buf: &[u8]) -> AxResult {
        copy_to_user(addr, buf).map_err(to_ax_error)
    }

    fn pin_user_pages(
        &self,
        addr: usize,
        len: usize,
        writable: bool,
    ) -> AxResult<control::PinnedUserPages> {
        pin_user_pages(addr, len, writable).map_err(to_ax_error)
    }

    fn release_pinned_user_pages(&self, id: control::PinnedUserPagesId) -> AxResult {
        release_pinned_user_pages(id).map_err(to_ax_error)
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

fn axvisor_user_fd_name(_: &dyn Inode) -> String {
    "anon_inode:[axvisor-user-fd]".to_string()
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
        let control_file = (ops.open)().map_err(from_ax_error)?;
        Ok(Box::new(KvmFile {
            control: KvmControlFile::new(control_file, ops),
        }))
    }
}

struct KvmFile {
    control: KvmControlFile,
}

impl Drop for KvmFile {
    fn drop(&mut self) {
        self.control.drop_file();
    }
}

impl FileOps for KvmFile {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "read is not supported by /dev/kvm");
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "write is not supported by /dev/kvm");
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
        self.control.ioctl(raw_ioctl)
    }
}

impl Pollable for KvmFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.control.poll(mask, poller)
    }
}

struct KvmControlFile {
    control_file: ControlFileId,
    ops: ControlOps,
}

impl KvmControlFile {
    fn new(control_file: ControlFileId, ops: ControlOps) -> Self {
        Self { control_file, ops }
    }

    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // Linux KVM does not expose poll-driven control readiness; treat the
        // control file like a regular ioctl-oriented fd for host poll/select.
        (IoEvents::IN | IoEvents::OUT) & mask
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        let result = (self.ops.ioctl)(self.control_file, raw_ioctl.cmd(), raw_ioctl.arg())
            .map_err(from_ax_error)?;
        i32::try_from(result)
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "ioctl return value overflow"))
    }

    fn drop_file(&self) {
        let _ = (self.ops.close)(self.control_file);
    }
}

struct KvmAnonFile {
    control: KvmControlFile,
    owns_file: AtomicBool,
    mmap_area: Option<Arc<Vmo>>,
    pseudo_path: Path,
}

impl KvmAnonFile {
    fn new(control_file: ControlFileId, ops: ControlOps, mmap_area: Option<Arc<Vmo>>) -> Self {
        Self {
            control: KvmControlFile::new(control_file, ops),
            owns_file: AtomicBool::new(false),
            mmap_area,
            pseudo_path: AnonInodeFs::new_path(axvisor_user_fd_name),
        }
    }

    fn adopt_file(&self) {
        self.owns_file.store(true, Ordering::Release);
    }
}

impl Drop for KvmAnonFile {
    fn drop(&mut self) {
        if self.owns_file.load(Ordering::Acquire) {
            self.control.drop_file();
        }
    }
}

impl Pollable for KvmAnonFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.control.poll(mask, poller)
    }
}

impl FileLike for KvmAnonFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        self.control.ioctl(raw_ioctl)
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn mappable(&self) -> Result<Mappable> {
        self.mmap_area
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

fn create_user_fd(
    control_file: ControlFileId,
    ops: ControlOps,
    mmap_area: Option<control::MmapAreaId>,
) -> Result<Fd> {
    let mmap_area = mmap_area.map(clone_mmap_area).transpose()?;
    let anon_file = Arc::new(KvmAnonFile::new(control_file, ops, mmap_area));
    match create_object_fd(anon_file.clone()) {
        Ok(fd) => {
            anon_file.adopt_file();
            Ok(fd)
        }
        Err(err) => Err(err),
    }
}

fn create_object_fd(file: Arc<dyn FileLike>) -> Result<Fd> {
    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    let file_table = thread_local.borrow_file_table();

    let fd = file_table.unwrap().write().insert(file, FdFlags::empty());
    Ok(fd.into())
}

fn copy_from_user(addr: usize, buf: &mut [u8]) -> Result<()> {
    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    CurrentUserSpace::new(thread_local).read_bytes(addr, buf)?;
    Ok(())
}

fn copy_to_user(addr: usize, buf: &[u8]) -> Result<()> {
    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    CurrentUserSpace::new(thread_local).write_bytes(addr, buf)?;
    Ok(())
}

fn get_user_fd_ref(fd: Fd) -> Result<control::UserFdRefId> {
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
    let user_fd_ref = next_user_fd_ref_id()?;
    USER_FD_REFS.lock().insert(user_fd_ref, file);
    Ok(user_fd_ref)
}

fn write_user_fd_ref(user_fd_ref: control::UserFdRefId, buf: &[u8]) -> Result<usize> {
    let file = USER_FD_REFS
        .lock()
        .get(&user_fd_ref)
        .cloned()
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user fd ref not found"))?;

    let mut reader = VmReader::from(buf).to_fallible();
    file.write(&mut reader)
}

fn read_user_fd_ref(user_fd_ref: control::UserFdRefId, buf: &mut [u8]) -> Result<usize> {
    let file = USER_FD_REFS
        .lock()
        .get(&user_fd_ref)
        .cloned()
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user fd ref not found"))?;

    loop {
        let mut writer = VmWriter::from(&mut *buf).to_fallible();
        match file.read(&mut writer) {
            Err(err) if err.error() == Errno::EAGAIN => {}
            result => return result,
        }

        let mut poller = Poller::new(None);
        if !file
            .poll(IoEvents::IN, Some(poller.as_handle_mut()))
            .is_empty()
        {
            continue;
        }
        poller.wait()?;
    }
}

fn clone_mmap_area(area: control::MmapAreaId) -> Result<Arc<Vmo>> {
    MMAP_AREAS
        .lock()
        .get(&area)
        .cloned()
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "mmap area not found"))
}

fn create_mmap_area(len: usize) -> Result<control::MmapAreaId> {
    if len == 0 {
        return_errno_with_message!(Errno::EINVAL, "mmap area length must be non-zero");
    }

    let area = next_mmap_area_id()?;
    MMAP_AREAS
        .lock()
        .insert(area, VmoOptions::new(len).alloc()?);
    Ok(area)
}

fn write_mmap_area(area: control::MmapAreaId, offset: usize, buf: &[u8]) -> Result<()> {
    let mmap_area = clone_mmap_area(area)?;
    mmap_area.write(offset, &mut VmReader::from(buf).to_fallible())?;
    Ok(())
}

fn read_mmap_area(area: control::MmapAreaId, offset: usize, buf: &mut [u8]) -> Result<()> {
    let mmap_area = clone_mmap_area(area)?;
    mmap_area.read(offset, &mut VmWriter::from(buf).to_fallible())?;
    Ok(())
}

fn pin_user_pages(addr: usize, len: usize, writable: bool) -> Result<control::PinnedUserPages> {
    if !addr.is_multiple_of(PAGE_SIZE) || !len.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "pinned user memory must be page aligned");
    }

    let task = Task::current()
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "current task is not available"))?;
    let thread_local = task
        .as_thread_local()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "current task is not a user thread"))?;
    let user_space = CurrentUserSpace::new(thread_local);
    let frames = user_space.vmar().acquire_pages_alien(addr, len, writable)?;
    let pages = frames.iter().map(|frame| frame.paddr().into()).collect();

    let id = next_pinned_user_pages_id()?;
    PINNED_USER_PAGES.lock().insert(id, frames);

    Ok(control::PinnedUserPages { id, pages })
}

fn release_mmap_area(area: control::MmapAreaId) -> Result<()> {
    MMAP_AREAS
        .lock()
        .remove(&area)
        .map(|_| ())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "mmap area not found"))
}

fn release_user_fd_ref(user_fd_ref: control::UserFdRefId) -> Result<()> {
    USER_FD_REFS
        .lock()
        .remove(&user_fd_ref)
        .map(|_| ())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "user fd ref not found"))
}

fn next_mmap_area_id() -> Result<control::MmapAreaId> {
    let area = NEXT_MMAP_AREA.fetch_add(1, Ordering::Relaxed);
    if area == 0 {
        return_errno_with_message!(Errno::EOVERFLOW, "mmap area id overflow");
    }
    Ok(area)
}

fn release_pinned_user_pages(id: control::PinnedUserPagesId) -> Result<()> {
    PINNED_USER_PAGES
        .lock()
        .remove(&id)
        .map(|_| ())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "pinned user pages not found"))
}

fn next_user_fd_ref_id() -> Result<control::UserFdRefId> {
    let user_fd = NEXT_USER_FD.fetch_add(1, Ordering::Relaxed);
    if user_fd == 0 {
        return_errno_with_message!(Errno::EOVERFLOW, "user fd ref id overflow");
    }
    Ok(user_fd)
}

fn next_pinned_user_pages_id() -> Result<control::PinnedUserPagesId> {
    let id = NEXT_PINNED_USER_PAGES.fetch_add(1, Ordering::Relaxed);
    if id == 0 {
        return_errno_with_message!(Errno::EOVERFLOW, "pinned user pages id overflow");
    }
    Ok(id)
}

fn endpoint_ops() -> Result<ControlOps> {
    KVM_ENDPOINT
        .lock()
        .as_ref()
        .copied()
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "the KVM endpoint is not registered"))
}

fn to_ax_error(err: Error) -> AxError {
    match err.error() {
        Errno::EEXIST => AxErrorKind::AlreadyExists.into(),
        Errno::EFAULT => AxErrorKind::BadAddress.into(),
        Errno::EINVAL => AxErrorKind::InvalidInput.into(),
        Errno::EAGAIN => AxErrorKind::WouldBlock.into(),
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
