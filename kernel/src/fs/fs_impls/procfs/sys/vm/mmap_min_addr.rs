// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    vm::vmar::VMAR_LOWEST_ADDR,
};

/// Represents the inode at `/proc/sys/vm/mmap_min_addr`.
pub struct MmapMinAddrFileOps;

impl MmapMinAddrFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for MmapMinAddrFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(printer, "{}", VMAR_LOWEST_ADDR)?;
        Ok(printer.bytes_written())
    }
}
