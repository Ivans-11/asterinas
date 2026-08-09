// SPDX-License-Identifier: MPL-2.0

.section .text.entry,"ax",@progbits
.global _start
.type _start,@function
.extern sandbox_c_main
_start:
	mov (%rsp), %rdi
	lea 8(%rsp), %rsi
	and $-16, %rsp
	call sandbox_c_main
	mov %eax, %edi
	mov $60, %eax
	syscall
1:
	jmp 1b

.section .note.GNU-stack,"",@progbits
