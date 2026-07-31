// SPDX-License-Identifier: MPL-2.0

.section .rodata.sandbox_guest,"a",@progbits
.balign 4096
.option norvc

.global sandbox_guest_start
.global sandbox_guest_entry
.global sandbox_guest_trap
.global sandbox_guest_user
.global sandbox_guest_end

sandbox_guest_start:
sandbox_guest_entry:
	li sp, 0xc000
	lla t0, sandbox_guest_user
	csrw sepc, t0
	li t0, 0x100
	csrc sstatus, t0
	sret

sandbox_guest_trap:
	csrr t0, scause
	li t1, 8
	beq t0, t1, sandbox_ecall
	li t1, 13
	beq t0, t1, sandbox_page_fault
	li t0, 0x10000000
	li t1, 'E'
	sb t1, 0(t0)
1:
	j 1b

sandbox_ecall:
	li t1, 0xd000
	sd t0, 0(t1)
	lla t0, sandbox_guest_attack
	csrw sepc, t0
	li t0, 0x10000000
	li t1, 'P'
	sb t1, 0(t0)
	sret

sandbox_page_fault:
	csrr t1, stval
	li t2, 0xd000
	sd t0, 8(t2)
	sd t1, 16(t2)
	li t0, 0x53414e44424f5821
	sd t0, 24(t2)
	li t0, 0x10000000
	li t1, 'Q'
	sb t1, 0(t0)
1:
	j 1b

.balign 4096
sandbox_guest_user:
	ecall

sandbox_guest_attack:
	li t0, 0x8000
	ld t1, 0(t0)
	li t0, 0x10000000
	li t1, 'F'
	sb t1, 0(t0)
1:
	j 1b

sandbox_guest_end:

.section .note.GNU-stack,"",@progbits
