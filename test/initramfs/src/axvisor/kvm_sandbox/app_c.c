// SPDX-License-Identifier: MPL-2.0

#if defined(__x86_64__)
#define SYS_WRITE 1

static long syscall3(long number, long argument0, long argument1, long argument2)
{
	long result;
	__asm__ volatile("syscall"
			 : "=a"(result)
			 : "a"(number), "D"(argument0), "S"(argument1), "d"(argument2)
			 : "rcx", "r11", "memory");
	return result;
}
#elif defined(__riscv)
#define SYS_WRITE 64

static long syscall3(long number, long argument0, long argument1, long argument2)
{
	register long a0 __asm__("a0") = argument0;
	register long a1 __asm__("a1") = argument1;
	register long a2 __asm__("a2") = argument2;
	register long a7 __asm__("a7") = number;
	__asm__ volatile("ecall" : "+r"(a0) : "r"(a1), "r"(a2), "r"(a7) : "memory");
	return a0;
}
#else
#error unsupported sandbox application architecture
#endif

static int string_equal(const char *left, const char *right)
{
	while (*left != '\0' && *right != '\0') {
		if (*left++ != *right++)
			return 0;
	}
	return *left == '\0' && *right == '\0';
}

int sandbox_c_main(long argument_count, char *arguments[])
{
	static const char message[] = "sandbox C application pass!\n";

	if (argument_count != 2 || !string_equal(arguments[1], "c-ok"))
		return 1;
	return syscall3(SYS_WRITE, 1, (long)message, sizeof(message) - 1) ==
		       (long)sizeof(message) - 1 ?
		       0 :
		       1;
}
