// SPDX-License-Identifier: MPL-2.0

use super::TidDirOps;
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, PerOpenFileOps, StatusFlags, mkmod},
        procfs::template::{ProcFile, ProcFileOpsByHandle},
        vfs::inode::{FileOps, Inode},
    },
    prelude::*,
    process::{
        VmarSnapshot,
        posix_thread::{AsPosixThread, alien_access::AlienAccessMode},
        signal::{PollHandle, Pollable},
    },
    thread::Thread,
    vm::vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR},
};

/// Represents the inode at `/proc/[pid]/task/[tid]/maps` (and also `/proc/[pid]/maps`).
pub struct MapsFileOps(TidDirOps);

impl MapsFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3343>
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOpsByHandle for MapsFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Result<Box<dyn PerOpenFileOps>> {
        let Some(process) = self.0.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };
        // Hold the process VMAR lock while checking access permissions and
        // taking the VMAR identity snapshot to prevent race conditions.
        let vmar_guard = process.lock_vmar();

        process
            .main_thread()
            .as_posix_thread()
            .unwrap()
            .check_alien_access_from(
                current_thread!().as_posix_thread().unwrap(),
                AlienAccessMode::READ_WITH_FS_CREDS,
            )
            .map_err(|_| Error::with_message(Errno::EACCES, "alien access is denied"))?;

        let vmar_snapshot = vmar_guard.snapshot();
        let Some(vmar) = vmar_guard.as_ref() else {
            return_errno_with_message!(Errno::ESRCH, "the process has exited");
        };

        let current = current_thread!();
        let fs_ref = current.as_posix_thread().unwrap().read_fs();
        let path_resolver = fs_ref.resolver().read();

        // Keep the generated contents stable for the lifetime of this file handle. Re-generating
        // the text for every partial read and skipping by byte offset can splice two different VMA
        // layouts together if the process maps or unmaps memory between reads.
        let heap_guard = vmar.process_vm().heap().lock();
        let guard = vmar.query(VMAR_LOWEST_ADDR..VMAR_CAP_ADDR);
        let mut contents = String::new();
        for vm_mapping in guard.iter() {
            vm_mapping.print_to_maps(&mut contents, vmar, &heap_guard, &path_resolver)?;
        }

        Ok(Box::new(MapsFileHandle {
            dir: self.0.clone(),
            vmar_snapshot,
            contents: contents.into_bytes(),
        }))
    }
}

/// A file handle opened from `/proc/[pid]/task/[tid]/maps` (and also `/proc/[pid]/maps`).
struct MapsFileHandle {
    dir: TidDirOps,
    vmar_snapshot: VmarSnapshot,
    contents: Vec<u8>,
}

impl Pollable for MapsFileHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for MapsFileHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let Some(process) = self.dir.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };
        let vmar_guard = process.lock_vmar();
        if !vmar_guard.is_same_as(&self.vmar_snapshot) {
            // The process has executed a new program.
            return Ok(0);
        }
        if vmar_guard.as_ref().is_none() {
            // The process has exited.
            return Ok(0);
        }

        let mut reader = VmReader::from(&self.contents[offset.min(self.contents.len())..]);
        Ok(writer.write_fallible(&mut reader)?)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "`/proc/[pid]/maps` is not writable");
    }
}

impl PerOpenFileOps for MapsFileHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}
