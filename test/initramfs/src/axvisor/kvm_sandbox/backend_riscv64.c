// SPDX-License-Identifier: MPL-2.0

#include "sandbox_abi.h"
#include "runner.h"
#include "elf_loader.h"
#include "syscall_proxy.h"

#define KVMIO 0xae
#define IOC(type, nr) (((type) << 8) | (nr))
#define IOC_WRITE 1UL
#define IOC_SIZESHIFT 16
#define IOC_DIRSHIFT 30
#define IOW(type, nr, size)                                                                   \
	((IOC_WRITE << IOC_DIRSHIFT) | ((size) << IOC_SIZESHIFT) | ((type) << 8) | (nr))

#define KVM_GET_API_VERSION IOC(KVMIO, 0x00)
#define KVM_CREATE_VM IOC(KVMIO, 0x01)
#define KVM_GET_VCPU_MMAP_SIZE IOC(KVMIO, 0x04)
#define KVM_CREATE_VCPU IOC(KVMIO, 0x41)
#define KVM_SET_USER_MEMORY_REGION IOW(KVMIO, 0x46, sizeof(struct kvm_userspace_memory_region))
#define KVM_RUN IOC(KVMIO, 0x80)
#define KVM_SET_ONE_REG IOW(KVMIO, 0xac, sizeof(struct kvm_one_reg))

#define KVM_EXIT_MMIO 6
#define KVM_EXIT_FAIL_ENTRY 9
#define KVM_EXIT_INTERNAL_ERROR 17

#define KVM_REG_RISCV 0x8000000000000000ULL
#define KVM_REG_SIZE_U64 0x0030000000000000ULL
#define KVM_REG_RISCV_CORE (0x02ULL << 24)
#define KVM_REG_RISCV_CSR (0x03ULL << 24)
#define KVM_REG_RISCV_CORE_REG(reg)                                                           \
	(KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_CORE | (reg))
#define KVM_REG_RISCV_CSR_REG(reg)                                                            \
	(KVM_REG_RISCV | KVM_REG_SIZE_U64 | KVM_REG_RISCV_CSR | (reg))

#define KVM_RISCV_CORE_PC 0
#define KVM_RISCV_CSR_SSTATUS 0
#define KVM_RISCV_CSR_STVEC 2
#define KVM_RISCV_CSR_SATP 8
#define EM_RISCV 243

#define AT_FDCWD -100
#define MAP_PRIVATE 0x02
#define MAP_SHARED 0x01
#define MAP_ANONYMOUS 0x20
#define O_RDWR 02
#define O_CLOEXEC 02000000
#define PROT_READ 0x1
#define PROT_WRITE 0x2

#define SYS_CLOSE 57
#define SYS_IOCTL 29
#define SYS_MMAP 222
#define SYS_OPENAT 56
#define SYS_WRITE 64

#define PAGE_SIZE 0x1000UL
#define GUEST_MEMORY_SIZE 0x200000UL
#define ROOT_PAGE_TABLE_GPA 0x1000UL
#define LEVEL1_PAGE_TABLE_GPA 0x2000UL
#define LEVEL0_PAGE_TABLE_GPA 0x3000UL
#define MONITOR_GPA 0x8000UL
#define APPLICATION_START_GPA 0x10000UL
#define USER_STACK_TOP_GPA 0x1fc000UL
#define RESULT_GPA 0xd000UL
#define MMIO_REPORT_GPA 0x10000000UL

#define SATP_MODE_SV39 (8ULL << 60)
#define SSTATUS_SPP (1ULL << 8)
#define SSTATUS_FS_INITIAL (1ULL << 13)
#define SSTATUS_SUM (1ULL << 18)

#define PTE_VALID (1ULL << 0)
#define PTE_READ (1ULL << 1)
#define PTE_WRITE (1ULL << 2)
#define PTE_EXECUTE (1ULL << 3)
#define PTE_USER (1ULL << 4)
#define PTE_ACCESSED (1ULL << 6)
#define PTE_DIRTY (1ULL << 7)

struct kvm_one_reg {
	unsigned long long id;
	unsigned long long addr;
};

struct kvm_run {
	unsigned char request_interrupt_window;
	unsigned char immediate_exit;
	unsigned char padding1[6];
	unsigned int exit_reason;
	unsigned char ready_for_interrupt_injection;
	unsigned char if_flag;
	unsigned short flags;
	unsigned long long cr8;
	unsigned long long apic_base;
	struct {
		unsigned long long phys_addr;
		unsigned char data[8];
		unsigned int len;
		unsigned char is_write;
	} mmio;
};

struct control_workspace {
	struct kvm_one_reg one_reg;
	unsigned long long value;
};

extern const unsigned char sandbox_monitor_start[];
extern const unsigned char sandbox_monitor_entry[];
extern const unsigned char sandbox_monitor_trap[];
extern const unsigned char sandbox_monitor_end[];

static unsigned char *guest_memory;

static long syscall3(long number, long argument0, long argument1, long argument2)
{
	register long a0 __asm__("a0") = argument0;
	register long a1 __asm__("a1") = argument1;
	register long a2 __asm__("a2") = argument2;
	register long a7 __asm__("a7") = number;

	__asm__ volatile("ecall" : "+r"(a0) : "r"(a1), "r"(a2), "r"(a7) : "memory");
	return a0;
}

static unsigned long string_length(const char *string)
{
	unsigned long length = 0;

	while (string[length] != '\0')
		length++;
	return length;
}

static void puts(const char *string)
{
	syscall3(SYS_WRITE, 1, (long)string, string_length(string));
}

static void copy_bytes(void *destination, const void *source, unsigned long length)
{
	unsigned char *output = destination;
	const unsigned char *input = source;

	while (length--)
		*output++ = *input++;
}

static int fail(const char *message)
{
	puts(message);
	return 1;
}

static unsigned long long page_table_entry(unsigned long physical_address,
					   unsigned long long flags)
{
	return ((unsigned long long)physical_address >> 2) | flags;
}

static void setup_guest_memory(const struct sandbox_image *image)
{
	unsigned long long *root = (unsigned long long *)&guest_memory[ROOT_PAGE_TABLE_GPA];
	unsigned long long *level1 = (unsigned long long *)&guest_memory[LEVEL1_PAGE_TABLE_GPA];
	unsigned long long *level0 = (unsigned long long *)&guest_memory[LEVEL0_PAGE_TABLE_GPA];
	unsigned long long data_flags =
		PTE_VALID | PTE_READ | PTE_WRITE | PTE_ACCESSED | PTE_DIRTY;

	root[0] = page_table_entry(LEVEL1_PAGE_TABLE_GPA, PTE_VALID);
	level1[0] = page_table_entry(LEVEL0_PAGE_TABLE_GPA, PTE_VALID);
	level1[(MMIO_REPORT_GPA >> 21) & 0x1ff] =
		page_table_entry(MMIO_REPORT_GPA, data_flags);

	for (unsigned long page = 0; page < GUEST_MEMORY_SIZE / PAGE_SIZE; page++) {
		unsigned long guest_address = page * PAGE_SIZE;
		unsigned long long flags = data_flags;

		if (guest_address == MONITOR_GPA)
			flags = PTE_VALID | PTE_READ | PTE_EXECUTE | PTE_ACCESSED;
		else if (page < sizeof(image->page_flags) && image->page_flags[page] != 0) {
			flags = PTE_VALID | PTE_READ | PTE_USER | PTE_ACCESSED;
			if (image->page_flags[page] & SANDBOX_IMAGE_PAGE_EXEC)
				flags |= PTE_EXECUTE;
			if (image->page_flags[page] & SANDBOX_IMAGE_PAGE_WRITE)
				flags |= PTE_WRITE | PTE_DIRTY;
		}
		else if (guest_address >= image->image_end &&
			 guest_address < USER_STACK_TOP_GPA - PAGE_SIZE &&
			 guest_address != RESULT_GPA && guest_address != SANDBOX_LAUNCH_ENTRY_GPA)
			flags = data_flags | PTE_USER;
		else if (guest_address == USER_STACK_TOP_GPA - PAGE_SIZE)
			flags = data_flags | PTE_USER;
		level0[page] = page_table_entry(guest_address, flags);
	}

}

static int set_one_reg(long vcpu_fd, struct control_workspace *workspace,
		       unsigned long long id, unsigned long long value)
{
	workspace->value = value;
	workspace->one_reg.id = id;
	workspace->one_reg.addr = (unsigned long long)&workspace->value;
	return syscall3(SYS_IOCTL, vcpu_fd, KVM_SET_ONE_REG, (long)&workspace->one_reg) == 0 ? 0 : 1;
}

static int setup_vcpu(long vcpu_fd, struct control_workspace *workspace, unsigned long entry_gpa,
		      unsigned long trap_gpa)
{
	unsigned long long satp = SATP_MODE_SV39 | (ROOT_PAGE_TABLE_GPA >> 12);

	if (set_one_reg(vcpu_fd, workspace, KVM_REG_RISCV_CORE_REG(KVM_RISCV_CORE_PC),
			entry_gpa) != 0)
		return fail("KVM_SET_ONE_REG pc failed\n");
	if (set_one_reg(vcpu_fd, workspace, KVM_REG_RISCV_CSR_REG(KVM_RISCV_CSR_SSTATUS),
			SSTATUS_SPP | SSTATUS_FS_INITIAL | SSTATUS_SUM) != 0)
		return fail("KVM_SET_ONE_REG sstatus failed\n");
	if (set_one_reg(vcpu_fd, workspace, KVM_REG_RISCV_CSR_REG(KVM_RISCV_CSR_STVEC),
			trap_gpa) != 0)
		return fail("KVM_SET_ONE_REG stvec failed\n");
	if (set_one_reg(vcpu_fd, workspace, KVM_REG_RISCV_CSR_REG(KVM_RISCV_CSR_SATP), satp) != 0)
		return fail("KVM_SET_ONE_REG satp failed\n");
	return 0;
}

static int decode_exit(struct sandbox_runner *runner, unsigned int *event_kind)
{
	struct kvm_run *run = runner->run;

	if (run->exit_reason != KVM_EXIT_MMIO || run->mmio.phys_addr != MMIO_REPORT_GPA ||
	    run->mmio.len != 1 || run->mmio.is_write != 1)
		return 1;
	if (run->mmio.data[0] == SANDBOX_REPORT_SYSCALL)
		*event_kind = SANDBOX_EVENT_SYSCALL;
	else if (run->mmio.data[0] == SANDBOX_REPORT_EXIT)
		*event_kind = SANDBOX_EVENT_EXIT;
	else if (run->mmio.data[0] == SANDBOX_REPORT_FAULT)
		*event_kind = SANDBOX_EVENT_FAULT;
	else
		return 1;
	return 0;
}

static const struct sandbox_linux_abi linux_abi = { .write_number = 64,
	.read_number = 63, .clock_gettime_number = 113, .brk_number = 214,
	.openat_number = 56, .close_number = 57, .lseek_number = 62,
	.set_tid_address_number = 96, .set_robust_list_number = 99, .rseq_number = 293,
	.rt_sigprocmask_number = 135,
	.fstat_number = 80, .fstat_size = 128, .prlimit64_number = 261, .readlinkat_number = 78,
	.mprotect_number = 226, .riscv_hwprobe_number = 258,
	.getrandom_number = 278,
	.exit_number = 93, .exit_group_number = 94 };

static int prepare_backend(struct sandbox_runner *runner,
			   const struct sandbox_image *image,
			   unsigned long long stack_pointer)
{
	unsigned long blob_size = (unsigned long)(sandbox_monitor_end - sandbox_monitor_start);
	unsigned long entry_gpa =
		MONITOR_GPA + (unsigned long)(sandbox_monitor_entry - sandbox_monitor_start);
	unsigned long trap_gpa =
		MONITOR_GPA + (unsigned long)(sandbox_monitor_trap - sandbox_monitor_start);
	struct control_workspace *workspace;

	if (blob_size > PAGE_SIZE)
		return fail("invalid sandbox guest layout\n");
	guest_memory = runner->guest_memory;
	copy_bytes(&guest_memory[MONITOR_GPA], sandbox_monitor_start, blob_size);
	setup_guest_memory(image);
	*(unsigned long long *)&guest_memory[SANDBOX_LAUNCH_ENTRY_GPA] = image->entry;
	*(unsigned long long *)&guest_memory[SANDBOX_LAUNCH_STACK_GPA] = stack_pointer;
	workspace = (struct control_workspace *)guest_memory;
	return setup_vcpu(runner->vcpu_fd, workspace, entry_gpa, trap_gpa);
}

const struct sandbox_backend_ops sandbox_backend = {
	.guest_memory_size = GUEST_MEMORY_SIZE,
	.application_start = APPLICATION_START_GPA,
	.stack_bottom = USER_STACK_TOP_GPA - SANDBOX_USER_STACK_SIZE,
	.stack_top = USER_STACK_TOP_GPA,
	.control_gpa = RESULT_GPA,
	.elf_machine = EM_RISCV,
	.linux_abi = &linux_abi,
	.prepare = prepare_backend,
	.decode_exit = decode_exit,
};
