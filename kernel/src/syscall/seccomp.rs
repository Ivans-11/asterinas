// SPDX-License-Identifier: MPL-2.0

//! Linux seccomp operations and classic-BPF filter installation.

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        seccomp::{
            BpfInstruction, SECCOMP_RET_ALLOW, SECCOMP_RET_ERRNO, SECCOMP_RET_KILL_PROCESS,
            SECCOMP_RET_KILL_THREAD, SECCOMP_RET_LOG, SECCOMP_RET_TRACE, SECCOMP_RET_TRAP,
            SeccompFilter,
        },
    },
};

const SECCOMP_SET_MODE_FILTER: u32 = 1;
const SECCOMP_GET_ACTION_AVAIL: u32 = 2;

const SECCOMP_FILTER_FLAG_TSYNC: u32 = 1;
const SECCOMP_MAX_INSNS_PER_FILTER: usize = 4096;

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct SockFprog {
    len: u16,
    filter: Vaddr,
}

pub fn sys_seccomp(
    operation: u32,
    flags: u32,
    args: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    match operation {
        SECCOMP_SET_MODE_FILTER => set_mode_filter(flags, args, ctx),
        SECCOMP_GET_ACTION_AVAIL => get_action_available(flags, args, ctx),
        _ => return_errno_with_message!(Errno::EINVAL, "unsupported seccomp operation"),
    }
}

fn set_mode_filter(flags: u32, program_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    if flags & !SECCOMP_FILTER_FLAG_TSYNC != 0 {
        return_errno_with_message!(Errno::EINVAL, "unsupported seccomp filter flags");
    }
    if !ctx.posix_thread.no_new_privs() {
        return_errno_with_message!(Errno::EACCES, "seccomp filters require no-new-privileges");
    }

    let program = ctx.user_space().read_val::<SockFprog>(program_addr)?;
    let instruction_count = program.len as usize;
    if instruction_count == 0 || instruction_count > SECCOMP_MAX_INSNS_PER_FILTER {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp BPF program length");
    }

    let mut instructions = vec![BpfInstruction::default(); instruction_count];
    ctx.user_space()
        .read_slice(program.filter, instructions.as_mut_slice())?;
    let filter = Arc::new(SeccompFilter::new(instructions)?);

    if flags & SECCOMP_FILTER_FLAG_TSYNC != 0 {
        let caller_state = ctx.posix_thread.seccomp_state();
        // Keep the task-set lock while installing the filter, so a concurrently
        // cloned thread cannot escape the synchronized filter state.
        let tasks = ctx.process.tasks().lock();
        if let Some(unsynchronized_thread) = tasks.as_slice().iter().find_map(|task| {
            let posix_thread = task.as_posix_thread().unwrap();
            (!posix_thread
                .seccomp_state()
                .is_filter_ancestor_of(&caller_state))
            .then_some(posix_thread)
        }) {
            // Linux reports the TID that prevented synchronization as a
            // successful (positive) seccomp return value.
            return Ok(SyscallReturn::Return(unsynchronized_thread.tid() as isize));
        }

        let mut synchronized_state = caller_state;
        synchronized_state.no_new_privs = true;
        synchronized_state.install_filter(filter);
        for task in tasks.as_slice() {
            let posix_thread = task.as_posix_thread().unwrap();
            posix_thread.replace_seccomp_state(synchronized_state.clone());
        }
    } else {
        ctx.posix_thread.install_seccomp_filter(filter);
    }

    Ok(SyscallReturn::Return(0))
}

fn get_action_available(flags: u32, action_addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    if flags != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp action query flags");
    }

    let action = ctx.user_space().read_val::<u32>(action_addr)?;
    match action {
        SECCOMP_RET_KILL_PROCESS
        | SECCOMP_RET_KILL_THREAD
        | SECCOMP_RET_TRAP
        | SECCOMP_RET_ERRNO
        | SECCOMP_RET_TRACE
        | SECCOMP_RET_LOG
        | SECCOMP_RET_ALLOW => Ok(SyscallReturn::Return(0)),
        _ => return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported seccomp action"),
    }
}
