// SPDX-License-Identifier: MPL-2.0

.section .text
.global _start
.type _start,@function
_start:
	.option push
	.option norelax
	lla gp, __global_pointer$
	.option pop
#ifdef SANDBOX_APP_MONITOR_ATTACK
	li t0, 0x8000
	ld t1, 0(t0)
	j sandbox_attack_unblocked
#elif defined(SANDBOX_APP_CONTROL_ATTACK)
	li t0, 0xd000
	ld t1, 0(t0)
	j sandbox_attack_unblocked
#elif defined(SANDBOX_APP_SPIN)
1:
	j 1b
#elif defined(SANDBOX_APP_FILE)
	ld t0, 0(sp)
	li t1, 2
	bne t0, t1, sandbox_app_failed
	li a0, -100
	ld a1, 16(sp)
	li a2, 0
	li a3, 0
	li a7, 56
	ecall
	bltz a0, sandbox_app_failed
	mv s0, a0
	addi sp, sp, -16
	mv a0, s0
	mv a1, sp
	li a2, 13
	li a7, 63
	ecall
	li t0, 13
	bne a0, t0, sandbox_app_failed
	lbu t0, 0(sp)
	li t1, 's'
	bne t0, t1, sandbox_app_failed
	lbu t0, 12(sp)
	li t1, 10
	bne t0, t1, sandbox_app_failed
	mv a0, s0
	li a1, 0
	li a2, 0
	li a7, 62
	ecall
	bnez a0, sandbox_app_failed
	mv a0, s0
	li a7, 57
	ecall
	bnez a0, sandbox_app_failed
	addi sp, sp, 16
	li a0, 1
	lla a1, sandbox_app_message
	li a2, 26
	li a7, 64
	ecall
	li a0, 0
	li a7, 93
	ecall
1:
	j 1b
sandbox_app_failed:
	li a0, 1
	li a7, 93
	ecall
	j 1b
sandbox_app_message:
	.ascii "sandbox file access pass!\n"
#else
#ifdef SANDBOX_APP_ALT
	ld t0, 0(sp)
	li t1, 2
	bne t0, t1, sandbox_app_failed
	ld t0, 16(sp)
	lbu t1, 0(t0)
	li t2, 'o'
	bne t1, t2, sandbox_app_failed
	lbu t1, 1(t0)
	li t2, 'k'
	bne t1, t2, sandbox_app_failed
	lbu t1, 2(t0)
	bnez t1, sandbox_app_failed
	lla t0, sandbox_app_counter
	ld t1, 0(t0)
	addi t1, t1, 1
	lla t0, sandbox_app_bss
	sd t1, 0(t0)
	li t0, 42
	bne t1, t0, sandbox_app_failed
	li a0, 0
	li a7, 214
	ecall
	mv s0, a0
	li t0, 4096
	add a0, a0, t0
	li a7, 214
	ecall
	li t0, 4096
	add t0, s0, t0
	bne a0, t0, sandbox_app_failed
	li t0, 0x5a
	sb t0, 0(s0)
	mv a0, s0
	li a7, 214
	ecall
	bne a0, s0, sandbox_app_failed
	addi sp, sp, -16
	li a0, 1
	mv a1, sp
	li a7, 113
	ecall
	addi sp, sp, 16
#endif
	li a0, 1
	lla a1, sandbox_app_message
	li a2, 24
	li a7, 64
	ecall
	li a0, 0
	li a7, 93
	ecall
1:
	j 1b

#ifdef SANDBOX_APP_ALT
sandbox_app_failed:
	li a0, 1
	li a7, 93
	ecall
	j 1b
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
	li a0, 0
	li a7, 93
	ecall
1:
	j 1b
#endif

.section .note.GNU-stack,"",@progbits
