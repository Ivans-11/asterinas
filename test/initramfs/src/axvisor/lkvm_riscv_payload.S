.section .text
.globl _start
_start:
    la a1, message

1:
    lbu a0, 0(a1)
    beqz a0, 2f
    li a7, 1
    ecall
    addi a1, a1, 1
    j 1b

2:
    li a0, 0
    li a1, 0
    li a6, 0
    li a7, 0x53525354
    ecall

3:
    wfi
    j 3b

.section .rodata
message:
    .asciz "lkvm riscv payload pass\n"
