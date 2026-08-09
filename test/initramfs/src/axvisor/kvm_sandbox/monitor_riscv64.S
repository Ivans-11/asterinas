// SPDX-License-Identifier: MPL-2.0

#include "sandbox_abi.h"

.section .rodata.sandbox_guest,"a",@progbits
.balign 4096
.option norvc

.global sandbox_monitor_start
.global sandbox_monitor_entry
.global sandbox_monitor_trap
.global sandbox_monitor_end

sandbox_monitor_start:
sandbox_monitor_entry:
	li t0, SANDBOX_LAUNCH_STACK_GPA
	ld sp, 0(t0)
	li t0, SANDBOX_LAUNCH_ENTRY_GPA
	ld t0, 0(t0)
	csrw sepc, t0
	li t0, 0x100
	csrc sstatus, t0
	sret

sandbox_monitor_trap:
	csrr t0, scause
	li t1, 8
	beq t0, t1, sandbox_ecall
	li t1, 2
	beq t0, t1, sandbox_page_fault
	li t1, 12
	beq t0, t1, sandbox_page_fault
	li t1, 13
	beq t0, t1, sandbox_page_fault
	li t1, 15
	beq t0, t1, sandbox_page_fault
	li t0, 0x10000000
	li t1, 'E'
	sb t1, 0(t0)
1:
	j 1b

sandbox_ecall:
	csrr t3, sepc
	addi t3, t3, 4
	csrw sepc, t3
	li t1, 93
	beq a7, t1, sandbox_exit
	li t1, 94
	beq a7, t1, sandbox_exit
	j sandbox_syscall

sandbox_syscall:
	li t1, 0xd000
	li t2, SANDBOX_MAGIC_VALUE
	sd t2, SANDBOX_CONTROL_MAGIC_OFFSET(t1)
	li t2, SANDBOX_ABI_VERSION
	sw t2, SANDBOX_CONTROL_VERSION_OFFSET(t1)
	li t2, SANDBOX_EVENT_SYSCALL
	sw t2, SANDBOX_CONTROL_EVENT_OFFSET(t1)
	ld t2, SANDBOX_CONTROL_SEQUENCE_OFFSET(t1)
	addi t2, t2, 1
	sd t2, SANDBOX_CONTROL_SEQUENCE_OFFSET(t1)
	sd a7, SANDBOX_CONTROL_NUMBER_OFFSET(t1)
	sd a0, SANDBOX_CONTROL_ARGS_OFFSET(t1)
	sd a1, SANDBOX_CONTROL_ARGS_OFFSET + 8(t1)
	sd a2, SANDBOX_CONTROL_ARGS_OFFSET + 16(t1)
	sd a3, SANDBOX_CONTROL_ARGS_OFFSET + 24(t1)
	sd a4, SANDBOX_CONTROL_ARGS_OFFSET + 32(t1)
	sd a5, SANDBOX_CONTROL_ARGS_OFFSET + 40(t1)
	fence rw, rw
	li t0, 0x10000000
	li t1, SANDBOX_REPORT_SYSCALL
	sb t1, 0(t0)
	li t1, 0xd000
	ld a0, SANDBOX_CONTROL_RESULT_OFFSET(t1)
	lw t2, SANDBOX_CONTROL_ACTION_OFFSET(t1)
	li t3, SANDBOX_ACTION_TERMINATE
	beq t2, t3, 1f
	sret

sandbox_exit:
	li t1, 0xd000
	li t2, SANDBOX_MAGIC_VALUE
	sd t2, SANDBOX_CONTROL_MAGIC_OFFSET(t1)
	li t2, SANDBOX_ABI_VERSION
	sw t2, SANDBOX_CONTROL_VERSION_OFFSET(t1)
	li t2, SANDBOX_EVENT_EXIT
	sw t2, SANDBOX_CONTROL_EVENT_OFFSET(t1)
	ld t2, SANDBOX_CONTROL_SEQUENCE_OFFSET(t1)
	addi t2, t2, 1
	sd t2, SANDBOX_CONTROL_SEQUENCE_OFFSET(t1)
	sd a7, SANDBOX_CONTROL_NUMBER_OFFSET(t1)
	sd a0, SANDBOX_CONTROL_ARGS_OFFSET(t1)
	fence rw, rw
	li t0, 0x10000000
	li t1, SANDBOX_REPORT_EXIT
	sb t1, 0(t0)
1:
	j 1b

sandbox_page_fault:
	csrr t0, scause
	csrr t1, stval
	li t2, 0xd000
	li t3, SANDBOX_MAGIC_VALUE
	sd t3, SANDBOX_CONTROL_MAGIC_OFFSET(t2)
	li t3, SANDBOX_ABI_VERSION
	sw t3, SANDBOX_CONTROL_VERSION_OFFSET(t2)
	li t3, SANDBOX_EVENT_FAULT
	sw t3, SANDBOX_CONTROL_EVENT_OFFSET(t2)
	ld t3, SANDBOX_CONTROL_SEQUENCE_OFFSET(t2)
	addi t3, t3, 1
	sd t3, SANDBOX_CONTROL_SEQUENCE_OFFSET(t2)
	sd t0, SANDBOX_CONTROL_ARGS_OFFSET(t2)
	sd t1, SANDBOX_CONTROL_ARGS_OFFSET + 8(t2)
	fence rw, rw
	li t0, 0x10000000
	li t1, SANDBOX_REPORT_FAULT
	sb t1, 0(t0)
1:
	j 1b

sandbox_monitor_end:

.section .note.GNU-stack,"",@progbits
