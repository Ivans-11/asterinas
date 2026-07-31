// SPDX-License-Identifier: MPL-2.0

.section .rodata.sandbox_guest,"a",@progbits
.balign 4096
.code64

.global sandbox_guest_start
.global sandbox_guest_entry
.global sandbox_guest_syscall
.global sandbox_guest_user
.global sandbox_guest_attack
.global sandbox_guest_end

sandbox_guest_start:
sandbox_guest_entry:
	mov $0x1b, %eax
	pushq %rax
	pushq %rsi
	pushq $0x2
	pushq $0x23
	pushq %rdi
	iretq

sandbox_guest_syscall:
	xor %eax, %eax
	mov %cs, %ax
	movabs $0xd000, %rbx
	mov %rax, 8(%rbx)
	movabs $0x53414e44424f5821, %rax
	mov %rax, 16(%rbx)
	mov $0xe9, %dx
	mov $'P', %al
	out %al, (%dx)
	mov $0x1b, %eax
	pushq %rax
	pushq %rsi
	pushq $0x2
	pushq $0x23
	pushq %rdi
	iretq

.balign 4096
sandbox_guest_user:
	xor %eax, %eax
	mov %cs, %ax
	movabs $0xd000, %rbx
	mov %rax, (%rbx)
	lea sandbox_guest_attack(%rip), %rdi
	mov $1, %eax
	syscall
	ud2

sandbox_guest_attack:
	movabs $0x8000, %rax
	mov (%rax), %rax
	mov $0xe9, %dx
	mov $'F', %al
	out %al, (%dx)
	hlt

sandbox_guest_end:

.section .note.GNU-stack,"",@progbits
