// SPDX-License-Identifier: MPL-2.0

#include "sandbox_abi.h"

.section .rodata.sandbox_guest,"a",@progbits
.balign 4096
.code64

.global sandbox_monitor_start
.global sandbox_monitor_entry
.global sandbox_monitor_syscall
.global sandbox_monitor_page_fault
.global sandbox_monitor_end

sandbox_monitor_start:
sandbox_monitor_entry:
	mov $0x1b, %eax
	pushq %rax
	pushq %rsi
	pushq $0x2
	pushq $0x23
	pushq %rdi
	iretq

sandbox_monitor_syscall:
	movq %r10, 0xe028
	movabs $0xe000, %r10
	mov %rsp, 0(%r10)
	mov %rcx, 8(%r10)
	mov %r11, 16(%r10)
	mov %rbx, 24(%r10)
	mov %r12, 32(%r10)
	mov $0x7800, %rsp
	movabs $0xd000, %rbx
	movabs $SANDBOX_MAGIC_VALUE, %r12
	mov %r12, SANDBOX_CONTROL_MAGIC_OFFSET(%rbx)
	movl $SANDBOX_ABI_VERSION, SANDBOX_CONTROL_VERSION_OFFSET(%rbx)
	cmp $60, %rax
	je sandbox_monitor_exit
	cmp $231, %rax
	je sandbox_monitor_exit
	jmp sandbox_monitor_syscall_event

sandbox_monitor_syscall_event:
	movl $SANDBOX_EVENT_SYSCALL, SANDBOX_CONTROL_EVENT_OFFSET(%rbx)
	mov SANDBOX_CONTROL_SEQUENCE_OFFSET(%rbx), %r12
	inc %r12
	mov %r12, SANDBOX_CONTROL_SEQUENCE_OFFSET(%rbx)
	mov %rax, SANDBOX_CONTROL_NUMBER_OFFSET(%rbx)
	mov %rdi, SANDBOX_CONTROL_ARGS_OFFSET(%rbx)
	mov %rsi, SANDBOX_CONTROL_ARGS_OFFSET + 8(%rbx)
	mov %rdx, SANDBOX_CONTROL_ARGS_OFFSET + 16(%rbx)
	movq 0xe028, %r12
	mov %r12, SANDBOX_CONTROL_ARGS_OFFSET + 24(%rbx)
	mov %r8, SANDBOX_CONTROL_ARGS_OFFSET + 32(%rbx)
	mov %r9, SANDBOX_CONTROL_ARGS_OFFSET + 40(%rbx)
	mov $0xe9, %dx
	mov $SANDBOX_REPORT_SYSCALL, %al
	out %al, (%dx)
	mov SANDBOX_CONTROL_RESULT_OFFSET(%rbx), %rax
	mov SANDBOX_CONTROL_ACTION_OFFSET(%rbx), %r12d
	cmp $SANDBOX_ACTION_TERMINATE, %r12d
	je sandbox_monitor_halt
	movabs $0xe000, %r10
	mov 0(%r10), %rsp
	mov 8(%r10), %rcx
	mov 16(%r10), %r11
	mov 24(%r10), %rbx
	mov 32(%r10), %r12
	movq 0xe028, %r10
	sysretq

sandbox_monitor_exit:
	movl $SANDBOX_EVENT_EXIT, SANDBOX_CONTROL_EVENT_OFFSET(%rbx)
	mov SANDBOX_CONTROL_SEQUENCE_OFFSET(%rbx), %r12
	inc %r12
	mov %r12, SANDBOX_CONTROL_SEQUENCE_OFFSET(%rbx)
	mov %rax, SANDBOX_CONTROL_NUMBER_OFFSET(%rbx)
	mov %rdi, SANDBOX_CONTROL_ARGS_OFFSET(%rbx)
	mov $0xe9, %dx
	mov $SANDBOX_REPORT_EXIT, %al
	out %al, (%dx)

sandbox_monitor_halt:
	hlt
	jmp sandbox_monitor_halt

sandbox_monitor_page_fault:
	movabs $0xd000, %rbx
	movabs $SANDBOX_MAGIC_VALUE, %rax
	mov %rax, SANDBOX_CONTROL_MAGIC_OFFSET(%rbx)
	movl $SANDBOX_ABI_VERSION, SANDBOX_CONTROL_VERSION_OFFSET(%rbx)
	movl $SANDBOX_EVENT_FAULT, SANDBOX_CONTROL_EVENT_OFFSET(%rbx)
	movq SANDBOX_CONTROL_SEQUENCE_OFFSET(%rbx), %rax
	inc %rax
	movq %rax, SANDBOX_CONTROL_SEQUENCE_OFFSET(%rbx)
	movq $14, SANDBOX_CONTROL_ARGS_OFFSET(%rbx)
	mov %cr2, %rax
	mov %rax, SANDBOX_CONTROL_ARGS_OFFSET + 8(%rbx)
	mov (%rsp), %rax
	mov %rax, SANDBOX_CONTROL_ARGS_OFFSET + 16(%rbx)
	mov $0xe9, %dx
	mov $SANDBOX_REPORT_FAULT, %al
	out %al, (%dx)
1:
	hlt
	jmp 1b

sandbox_monitor_end:

.section .note.GNU-stack,"",@progbits
