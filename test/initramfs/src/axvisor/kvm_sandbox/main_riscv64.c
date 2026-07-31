// SPDX-License-Identifier: MPL-2.0

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
#define SYS_EXIT 93

#define PAGE_SIZE 0x1000UL
#define GUEST_MEMORY_SIZE 0x20000UL
#define ROOT_PAGE_TABLE_GPA 0x1000UL
#define LEVEL1_PAGE_TABLE_GPA 0x2000UL
#define LEVEL0_PAGE_TABLE_GPA 0x3000UL
#define MONITOR_GPA 0x8000UL
#define USER_STACK_TOP_GPA 0xc000UL
#define RESULT_GPA 0xd000UL
#define MMIO_REPORT_GPA 0x10000000UL
#define SANDBOX_MAGIC 0x53414e44424f5821ULL

#define SATP_MODE_SV39 (8ULL << 60)
#define SSTATUS_SPP (1ULL << 8)
#define SSTATUS_SUM (1ULL << 18)

#define PTE_VALID (1ULL << 0)
#define PTE_READ (1ULL << 1)
#define PTE_WRITE (1ULL << 2)
#define PTE_EXECUTE (1ULL << 3)
#define PTE_USER (1ULL << 4)
#define PTE_ACCESSED (1ULL << 6)
#define PTE_DIRTY (1ULL << 7)

struct kvm_userspace_memory_region {
	unsigned int slot;
	unsigned int flags;
	unsigned long long guest_phys_addr;
	unsigned long long memory_size;
	unsigned long long userspace_addr;
};

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

struct sandbox_result {
	unsigned long long ecall_cause;
	unsigned long long page_fault_cause;
	unsigned long long fault_address;
	unsigned long long magic;
};

struct control_workspace {
	struct kvm_userspace_memory_region region;
	struct kvm_one_reg one_reg;
	unsigned long long value;
};

extern const unsigned char sandbox_guest_start[];
extern const unsigned char sandbox_guest_entry[];
extern const unsigned char sandbox_guest_trap[];
extern const unsigned char sandbox_guest_user[];
extern const unsigned char sandbox_guest_end[];

static unsigned char *guest_memory;

static long syscall1(long number, long argument0)
{
	register long a0 __asm__("a0") = argument0;
	register long a7 __asm__("a7") = number;

	__asm__ volatile("ecall" : "+r"(a0) : "r"(a7) : "memory");
	return a0;
}

static long syscall3(long number, long argument0, long argument1, long argument2)
{
	register long a0 __asm__("a0") = argument0;
	register long a1 __asm__("a1") = argument1;
	register long a2 __asm__("a2") = argument2;
	register long a7 __asm__("a7") = number;

	__asm__ volatile("ecall" : "+r"(a0) : "r"(a1), "r"(a2), "r"(a7) : "memory");
	return a0;
}

static long syscall6(long number, long argument0, long argument1, long argument2, long argument3,
		     long argument4, long argument5)
{
	register long a0 __asm__("a0") = argument0;
	register long a1 __asm__("a1") = argument1;
	register long a2 __asm__("a2") = argument2;
	register long a3 __asm__("a3") = argument3;
	register long a4 __asm__("a4") = argument4;
	register long a5 __asm__("a5") = argument5;
	register long a7 __asm__("a7") = number;

	__asm__ volatile("ecall"
			 : "+r"(a0)
			 : "r"(a1), "r"(a2), "r"(a3), "r"(a4), "r"(a5), "r"(a7)
			 : "memory");
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

static void zero_bytes(void *destination, unsigned long length)
{
	unsigned char *output = destination;

	while (length--)
		*output++ = 0;
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

static void setup_guest_memory(unsigned long payload_gpa)
{
	unsigned long long *root = (unsigned long long *)&guest_memory[ROOT_PAGE_TABLE_GPA];
	unsigned long long *level1 = (unsigned long long *)&guest_memory[LEVEL1_PAGE_TABLE_GPA];
	unsigned long long *level0 = (unsigned long long *)&guest_memory[LEVEL0_PAGE_TABLE_GPA];
	unsigned long payload_page = payload_gpa & ~(PAGE_SIZE - 1);
	unsigned long long leaf_flags =
		PTE_VALID | PTE_READ | PTE_WRITE | PTE_EXECUTE | PTE_ACCESSED | PTE_DIRTY;

	zero_bytes(guest_memory, GUEST_MEMORY_SIZE);
	root[0] = page_table_entry(LEVEL1_PAGE_TABLE_GPA, PTE_VALID);
	level1[0] = page_table_entry(LEVEL0_PAGE_TABLE_GPA, PTE_VALID);
	level1[(MMIO_REPORT_GPA >> 21) & 0x1ff] =
		page_table_entry(MMIO_REPORT_GPA, leaf_flags | PTE_USER);

	for (unsigned long page = 0; page < GUEST_MEMORY_SIZE / PAGE_SIZE; page++) {
		unsigned long guest_address = page * PAGE_SIZE;
		unsigned long long flags = leaf_flags;

		if (guest_address == payload_page ||
		    guest_address == USER_STACK_TOP_GPA - PAGE_SIZE || guest_address == RESULT_GPA)
			flags |= PTE_USER;
		level0[page] = page_table_entry(guest_address, flags);
	}

	copy_bytes(&guest_memory[MONITOR_GPA], sandbox_guest_start,
		   (unsigned long)(sandbox_guest_end - sandbox_guest_start));
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
			SSTATUS_SPP | SSTATUS_SUM) != 0)
		return fail("KVM_SET_ONE_REG sstatus failed\n");
	if (set_one_reg(vcpu_fd, workspace, KVM_REG_RISCV_CSR_REG(KVM_RISCV_CSR_STVEC),
			trap_gpa) != 0)
		return fail("KVM_SET_ONE_REG stvec failed\n");
	if (set_one_reg(vcpu_fd, workspace, KVM_REG_RISCV_CSR_REG(KVM_RISCV_CSR_SATP), satp) != 0)
		return fail("KVM_SET_ONE_REG satp failed\n");
	return 0;
}

static int expect_mmio_report(struct kvm_run *run, unsigned char report)
{
	return run->exit_reason == KVM_EXIT_MMIO && run->mmio.phys_addr == MMIO_REPORT_GPA &&
	       run->mmio.len == 1 && run->mmio.is_write == 1 && run->mmio.data[0] == report;
}

static int run_sandbox(void)
{
	unsigned long blob_size = (unsigned long)(sandbox_guest_end - sandbox_guest_start);
	unsigned long entry_gpa =
		MONITOR_GPA + (unsigned long)(sandbox_guest_entry - sandbox_guest_start);
	unsigned long trap_gpa =
		MONITOR_GPA + (unsigned long)(sandbox_guest_trap - sandbox_guest_start);
	unsigned long payload_gpa =
		MONITOR_GPA + (unsigned long)(sandbox_guest_user - sandbox_guest_start);
	struct control_workspace *workspace;
	struct sandbox_result *result;
	struct kvm_run *run;
	long kvm_fd, vm_fd, vcpu_fd, mmap_size;

	if (blob_size > 2 * PAGE_SIZE || (payload_gpa & (PAGE_SIZE - 1)) != 0)
		return fail("invalid sandbox guest layout\n");
	puts("kvm sandbox: start\n");

	kvm_fd = syscall3(SYS_OPENAT, AT_FDCWD, (long)"/dev/kvm", O_RDWR | O_CLOEXEC);
	if (kvm_fd < 0)
		return fail("open /dev/kvm failed\n");
	if (syscall3(SYS_IOCTL, kvm_fd, KVM_GET_API_VERSION, 0) != 12)
		return fail("KVM_GET_API_VERSION failed\n");
	mmap_size = syscall3(SYS_IOCTL, kvm_fd, KVM_GET_VCPU_MMAP_SIZE, 0);
	if (mmap_size < (long)sizeof(struct kvm_run))
		return fail("KVM_GET_VCPU_MMAP_SIZE failed\n");
	puts("kvm sandbox: KVM device ready\n");

	vm_fd = syscall3(SYS_IOCTL, kvm_fd, KVM_CREATE_VM, 0);
	if (vm_fd < 0)
		return fail("KVM_CREATE_VM failed\n");
	guest_memory = (unsigned char *)syscall6(SYS_MMAP, 0, GUEST_MEMORY_SIZE,
						 PROT_READ | PROT_WRITE,
						 MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if ((long)guest_memory < 0)
		return fail("mmap guest memory failed\n");

	setup_guest_memory(payload_gpa);
	result = (struct sandbox_result *)&guest_memory[RESULT_GPA];
	workspace = (struct control_workspace *)guest_memory;
	workspace->region.slot = 0;
	workspace->region.flags = 0;
	workspace->region.guest_phys_addr = 0;
	workspace->region.memory_size = GUEST_MEMORY_SIZE;
	workspace->region.userspace_addr = (unsigned long long)guest_memory;
	if (syscall3(SYS_IOCTL, vm_fd, KVM_SET_USER_MEMORY_REGION, (long)&workspace->region) != 0)
		return fail("KVM_SET_USER_MEMORY_REGION failed\n");

	vcpu_fd = syscall3(SYS_IOCTL, vm_fd, KVM_CREATE_VCPU, 0);
	if (vcpu_fd < 0)
		return fail("KVM_CREATE_VCPU failed\n");
	run = (struct kvm_run *)syscall6(SYS_MMAP, 0, mmap_size, PROT_READ | PROT_WRITE,
						MAP_SHARED, vcpu_fd, 0);
	if ((long)run < 0)
		return fail("mmap KVM vCPU failed\n");
	if (setup_vcpu(vcpu_fd, workspace, entry_gpa, trap_gpa) != 0)
		return 1;
	puts("kvm sandbox: guest ready\n");

	if (syscall3(SYS_IOCTL, vcpu_fd, KVM_RUN, 0) != 0)
		return fail("KVM_RUN failed\n");
	if (!expect_mmio_report(run, 'P'))
		return fail("sandbox privilege transition failed\n");
	if (result->ecall_cause != 8)
		return fail("sandbox ecall handling failed\n");

	if (syscall3(SYS_IOCTL, vcpu_fd, KVM_RUN, 0) != 0)
		return fail("second KVM_RUN failed\n");
	if (!expect_mmio_report(run, 'Q'))
		return fail("sandbox page isolation failed\n");
	if (result->page_fault_cause != 13 || result->fault_address != MONITOR_GPA ||
	    result->magic != SANDBOX_MAGIC)
		return fail("sandbox page fault state failed\n");

	syscall1(SYS_CLOSE, vcpu_fd);
	syscall1(SYS_CLOSE, vm_fd);
	syscall1(SYS_CLOSE, kvm_fd);
	puts("kvm sandbox pass\n");
	return 0;
}

void _start(void)
{
	syscall1(SYS_EXIT, run_sandbox());
	for (;;) {
	}
}
