// SPDX-License-Identifier: MPL-2.0

.section .text
.global _start
.type _start,@function
_start:
#ifdef SANDBOX_APP_MONITOR_ATTACK
	movabs $0x8000, %rax
	mov (%rax), %rax
	jmp sandbox_attack_unblocked
#elif defined(SANDBOX_APP_CONTROL_ATTACK)
	movabs $0xd000, %rax
	mov (%rax), %rax
	jmp sandbox_attack_unblocked
#elif defined(SANDBOX_APP_SPIN)
1:
	jmp 1b
#elif defined(SANDBOX_APP_FILE)
	cmpq $2, (%rsp)
	jne sandbox_app_failed
	mov $257, %eax
	mov $-100, %edi
	mov 16(%rsp), %rsi
	xor %edx, %edx
	xor %r10d, %r10d
	syscall
	test %rax, %rax
	js sandbox_app_failed
	mov %rax, %r12
	sub $16, %rsp
	xor %eax, %eax
	mov %r12, %rdi
	mov %rsp, %rsi
	mov $13, %edx
	syscall
	cmp $13, %rax
	jne sandbox_app_failed
	cmpb $'s', 0(%rsp)
	jne sandbox_app_failed
	cmpb $10, 12(%rsp)
	jne sandbox_app_failed
	mov $8, %eax
	mov %r12, %rdi
	xor %esi, %esi
	xor %edx, %edx
	syscall
	test %rax, %rax
	jne sandbox_app_failed
	mov $3, %eax
	mov %r12, %rdi
	syscall
	test %rax, %rax
	jne sandbox_app_failed
	add $16, %rsp
	mov $1, %eax
	mov $1, %edi
	lea sandbox_app_message(%rip), %rsi
	mov $26, %edx
	syscall
	mov $60, %eax
	xor %edi, %edi
	syscall
	jmp .
sandbox_app_failed:
	mov $60, %eax
	mov $1, %edi
	syscall
	jmp .
sandbox_app_message:
	.ascii "sandbox file access pass!\n"
#else
#ifdef SANDBOX_APP_ALT
	cmpq $2, (%rsp)
	jne sandbox_app_failed
	mov 16(%rsp), %rax
	cmpb $'o', 0(%rax)
	jne sandbox_app_failed
	cmpb $'k', 1(%rax)
	jne sandbox_app_failed
	cmpb $0, 2(%rax)
	jne sandbox_app_failed
	mov sandbox_app_counter(%rip), %rax
	inc %rax
	mov %rax, sandbox_app_bss(%rip)
	cmpq $42, sandbox_app_bss(%rip)
	jne sandbox_app_failed
	mov $12, %eax
	xor %edi, %edi
	syscall
	mov %rax, %r12
	lea 4096(%rax), %rdi
	mov $12, %eax
	syscall
	cmp %rdi, %rax
	jne sandbox_app_failed
	movb $0x5a, (%r12)
	mov %r12, %rdi
	mov $12, %eax
	syscall
	cmp %r12, %rax
	jne sandbox_app_failed
	sub $16, %rsp
	mov $1, %edi
	mov %rsp, %rsi
	mov $228, %eax
	syscall
	add $16, %rsp
#endif
	mov $1, %eax
	mov $1, %edi
	lea sandbox_app_message(%rip), %rsi
	mov $24, %edx
	syscall
	mov $60, %eax
	xor %edi, %edi
	syscall
	jmp .

#ifdef SANDBOX_APP_ALT
sandbox_app_failed:
	mov $60, %eax
	mov $1, %edi
	syscall
	jmp .
#endif

sandbox_app_message:
#ifdef SANDBOX_APP_ALT
	.ascii "second ELF application!\n"
#else
	.ascii "sandbox ELF says hello!\n"
#endif

#ifdef SANDBOX_APP_ALT
.section .data
.balign 8
sandbox_app_counter:
	.quad 41

.section .bss
.balign 8
sandbox_app_bss:
	.zero 8
#endif
#endif

#if defined(SANDBOX_APP_MONITOR_ATTACK) || defined(SANDBOX_APP_CONTROL_ATTACK)
sandbox_attack_unblocked:
	mov $60, %eax
	xor %edi, %edi
	syscall
	jmp .
#endif

.section .note.GNU-stack,"",@progbits
