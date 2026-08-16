// SPDX-License-Identifier: MPL-2.0

//! Seccomp filter state and classic-BPF execution.

use crate::prelude::*;

const BPF_MAX_INSNS: usize = 4096;

const BPF_LD_W_ABS: u16 = 0x20;
const BPF_ALU_AND_K: u16 = 0x54;
const BPF_JMP_JA: u16 = 0x05;
const BPF_JMP_JEQ_K: u16 = 0x15;
const BPF_JMP_JGT_K: u16 = 0x25;
const BPF_JMP_JGE_K: u16 = 0x35;
const BPF_JMP_JSET_K: u16 = 0x45;
const BPF_RET_K: u16 = 0x06;

pub const SECCOMP_RET_KILL_THREAD: u32 = 0x0000_0000;
pub const SECCOMP_RET_TRAP: u32 = 0x0003_0000;
pub const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
pub const SECCOMP_RET_TRACE: u32 = 0x7ff0_0000;
pub const SECCOMP_RET_LOG: u32 = 0x7ffc_0000;
pub const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
pub const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;

const SECCOMP_RET_ACTION_FULL: u32 = 0xffff_0000;
const SECCOMP_RET_DATA: u32 = 0x0000_ffff;

/// A classic-BPF instruction as defined by `linux/filter.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct BpfInstruction {
    pub code: u16,
    pub jump_if_true: u8,
    pub jump_if_false: u8,
    pub constant: u32,
}

/// The syscall metadata exposed to a seccomp filter.
pub struct SeccompData {
    syscall_number: i32,
    audit_arch: u32,
    instruction_pointer: u64,
    args: [u64; 6],
}

impl SeccompData {
    pub fn new(syscall_number: u64, instruction_pointer: usize, args: [u64; 6]) -> Self {
        Self {
            syscall_number: syscall_number as i32,
            audit_arch: native_audit_arch(),
            instruction_pointer: instruction_pointer as u64,
            args,
        }
    }

    fn load_word(&self, offset: u32) -> Option<u32> {
        match offset {
            0 => Some(self.syscall_number as u32),
            4 => Some(self.audit_arch),
            8 => Some(self.instruction_pointer as u32),
            12 => Some((self.instruction_pointer >> 32) as u32),
            16..64 if offset.is_multiple_of(4) => {
                let args_offset = offset - 16;
                let arg = self.args[(args_offset / 8) as usize];
                if args_offset.is_multiple_of(8) {
                    Some(arg as u32)
                } else {
                    Some((arg >> 32) as u32)
                }
            }
            _ => None,
        }
    }
}

/// A validated seccomp classic-BPF program.
#[derive(Debug)]
pub struct SeccompFilter {
    instructions: Vec<BpfInstruction>,
}

impl SeccompFilter {
    pub fn new(instructions: Vec<BpfInstruction>) -> Result<Self> {
        validate_program(&instructions)?;
        Ok(Self { instructions })
    }

    fn execute(&self, data: &SeccompData) -> u32 {
        let mut accumulator = 0_u32;
        let mut pc = 0_usize;

        loop {
            let instruction = self.instructions[pc];
            match instruction.code {
                BPF_LD_W_ABS => {
                    // The verifier has already checked this offset.
                    accumulator = data.load_word(instruction.constant).unwrap();
                }
                BPF_ALU_AND_K => accumulator &= instruction.constant,
                BPF_JMP_JA => pc += instruction.constant as usize,
                BPF_JMP_JEQ_K => {
                    pc += conditional_offset(&instruction, accumulator == instruction.constant)
                }
                BPF_JMP_JGT_K => {
                    pc += conditional_offset(&instruction, accumulator > instruction.constant)
                }
                BPF_JMP_JGE_K => {
                    pc += conditional_offset(&instruction, accumulator >= instruction.constant)
                }
                BPF_JMP_JSET_K => {
                    pc += conditional_offset(&instruction, accumulator & instruction.constant != 0)
                }
                BPF_RET_K => return instruction.constant,
                _ => unreachable!("the seccomp program was validated"),
            }
            pc += 1;
        }
    }
}

/// The result of evaluating all filters attached to a thread.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeccompAction {
    Allow,
    Errno(u16),
    KillProcess,
    KillThread,
    Log,
    Trace(u16),
    Trap(u16),
}

impl SeccompAction {
    fn from_raw(raw_action: u32) -> Self {
        let data = (raw_action & SECCOMP_RET_DATA) as u16;
        match raw_action & SECCOMP_RET_ACTION_FULL {
            SECCOMP_RET_ALLOW => Self::Allow,
            SECCOMP_RET_ERRNO => Self::Errno(data),
            SECCOMP_RET_KILL_PROCESS => Self::KillProcess,
            SECCOMP_RET_KILL_THREAD => Self::KillThread,
            SECCOMP_RET_LOG => Self::Log,
            SECCOMP_RET_TRACE => Self::Trace(data),
            SECCOMP_RET_TRAP => Self::Trap(data),
            // An unknown action kills the process, matching Linux's fail-closed behavior.
            _ => Self::KillProcess,
        }
    }

    fn precedence(self) -> u8 {
        match self {
            Self::KillProcess => 0,
            Self::KillThread => 1,
            Self::Trap(_) => 2,
            Self::Errno(_) => 3,
            Self::Trace(_) => 4,
            Self::Log => 5,
            Self::Allow => 6,
        }
    }
}

/// Per-thread seccomp state inherited by clone and preserved across exec.
#[derive(Clone, Default)]
pub struct SeccompState {
    pub no_new_privs: bool,
    filters: Vec<Arc<SeccompFilter>>,
}

impl SeccompState {
    pub fn install_filter(&mut self, filter: Arc<SeccompFilter>) {
        self.filters.push(filter);
    }

    pub fn evaluate(&self, data: &SeccompData) -> SeccompAction {
        self.filters
            .iter()
            .map(|filter| SeccompAction::from_raw(filter.execute(data)))
            .min_by_key(|action| action.precedence())
            .unwrap_or(SeccompAction::Allow)
    }

    pub fn is_filter_ancestor_of(&self, descendant: &Self) -> bool {
        self.filters.len() <= descendant.filters.len()
            && self
                .filters
                .iter()
                .zip(&descendant.filters)
                .all(|(left, right)| Arc::ptr_eq(left, right))
    }
}

fn conditional_offset(instruction: &BpfInstruction, condition: bool) -> usize {
    if condition {
        instruction.jump_if_true as usize
    } else {
        instruction.jump_if_false as usize
    }
}

fn validate_program(instructions: &[BpfInstruction]) -> Result<()> {
    if instructions.is_empty() || instructions.len() > BPF_MAX_INSNS {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp BPF program length");
    }
    if instructions.last().unwrap().code != BPF_RET_K {
        return_errno_with_message!(Errno::EINVAL, "seccomp BPF program does not return");
    }

    for (pc, instruction) in instructions.iter().enumerate() {
        let next_pc = pc + 1;
        match instruction.code {
            BPF_LD_W_ABS if instruction.constant.is_multiple_of(4) && instruction.constant < 64 => {
            }
            BPF_ALU_AND_K | BPF_RET_K => {}
            BPF_JMP_JA => {
                let target = next_pc.checked_add(instruction.constant as usize);
                if target.is_none_or(|target| target >= instructions.len()) {
                    return_errno_with_message!(Errno::EINVAL, "invalid seccomp BPF jump");
                }
            }
            BPF_JMP_JEQ_K | BPF_JMP_JGT_K | BPF_JMP_JGE_K | BPF_JMP_JSET_K => {
                let true_target = next_pc + instruction.jump_if_true as usize;
                let false_target = next_pc + instruction.jump_if_false as usize;
                if true_target >= instructions.len() || false_target >= instructions.len() {
                    return_errno_with_message!(Errno::EINVAL, "invalid seccomp BPF jump");
                }
            }
            _ => return_errno_with_message!(Errno::EINVAL, "unsupported seccomp BPF instruction"),
        }
    }

    Ok(())
}

#[cfg(target_arch = "x86_64")]
const fn native_audit_arch() -> u32 {
    0xc000_003e
}

#[cfg(target_arch = "riscv64")]
const fn native_audit_arch() -> u32 {
    0xc000_00f3
}

#[cfg(target_arch = "loongarch64")]
const fn native_audit_arch() -> u32 {
    0xc000_0102
}
