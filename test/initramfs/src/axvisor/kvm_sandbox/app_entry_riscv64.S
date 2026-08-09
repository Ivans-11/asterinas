// SPDX-License-Identifier: MPL-2.0

.section .text.entry,"ax",@progbits
.global _start
.type _start,@function
.extern sandbox_c_main
_start:
	.option push
	.option norelax
	lla gp, __global_pointer$
	.option pop
	ld a0, 0(sp)
	addi a1, sp, 8
	andi sp, sp, -16
	call sandbox_c_main
	li a7, 93
	ecall
1:
	j 1b

.section .note.GNU-stack,"",@progbits
