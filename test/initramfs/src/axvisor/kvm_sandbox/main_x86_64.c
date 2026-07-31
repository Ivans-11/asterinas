// SPDX-License-Identifier: MPL-2.0

#define KVMIO 0xae
#define IOC(type, nr) (((type) << 8) | (nr))
#define IOC_WRITE 1UL
#define IOC_READ 2UL
#define IOC_TYPESHIFT 8
#define IOC_SIZESHIFT 16
#define IOC_DIRSHIFT 30
#define IOR(type, nr, size)                                                                  \
	((IOC_READ << IOC_DIRSHIFT) | ((size) << IOC_SIZESHIFT) | ((type) << IOC_TYPESHIFT) |  \
	 (nr))
#define IOW(type, nr, size)                                                                   \
	((IOC_WRITE << IOC_DIRSHIFT) | ((size) << IOC_SIZESHIFT) | ((type) << IOC_TYPESHIFT) | \
	 (nr))

#define KVM_GET_API_VERSION IOC(KVMIO, 0x00)
#define KVM_CREATE_VM IOC(KVMIO, 0x01)
#define KVM_GET_VCPU_MMAP_SIZE IOC(KVMIO, 0x04)
#define KVM_CREATE_VCPU IOC(KVMIO, 0x41)
#define KVM_SET_USER_MEMORY_REGION IOW(KVMIO, 0x46, sizeof(struct kvm_userspace_memory_region))
#define KVM_RUN IOC(KVMIO, 0x80)
#define KVM_SET_REGS IOW(KVMIO, 0x82, sizeof(struct kvm_regs))
#define KVM_GET_SREGS IOR(KVMIO, 0x83, sizeof(struct kvm_sregs))
#define KVM_SET_SREGS IOW(KVMIO, 0x84, sizeof(struct kvm_sregs))
#define KVM_SET_MSRS IOW(KVMIO, 0x89, sizeof(struct kvm_msrs_header))

#define KVM_EXIT_IO 2
#define KVM_EXIT_SHUTDOWN 8
#define KVM_EXIT_IO_OUT 1

#define KVM_MSR_STAR 0xc0000081U
#define KVM_MSR_LSTAR 0xc0000082U
#define KVM_MSR_SYSCALL_MASK 0xc0000084U

#define AT_FDCWD -100
#define MAP_PRIVATE 0x02
#define MAP_SHARED 0x01
#define MAP_ANONYMOUS 0x20
#define O_RDWR 02
#define O_CLOEXEC 02000000
#define PROT_READ 0x1
#define PROT_WRITE 0x2

#define SYS_WRITE 1
#define SYS_CLOSE 3
#define SYS_IOCTL 16
#define SYS_MMAP 9
#define SYS_OPENAT 257
#define SYS_EXIT 60

#define PAGE_SIZE 0x1000UL
#define GUEST_MEMORY_SIZE 0x20000UL
#define PML4_GPA 0x1000UL
#define PDPT_GPA 0x2000UL
#define PD_GPA 0x3000UL
#define PT_GPA 0x4000UL
#define GDT_GPA 0x5000UL
#define MONITOR_GPA 0x8000UL
#define USER_STACK_TOP_GPA 0xc000UL
#define RESULT_GPA 0xd000UL
#define SANDBOX_MAGIC 0x53414e44424f5821ULL

#define CR0_PE (1UL << 0)
#define CR0_MP (1UL << 1)
#define CR0_ET (1UL << 4)
#define CR0_NE (1UL << 5)
#define CR0_WP (1UL << 16)
#define CR0_PG (1UL << 31)
#define CR4_PAE (1UL << 5)
#define EFER_SCE (1UL << 0)
#define EFER_LME (1UL << 8)
#define EFER_LMA (1UL << 10)

#define PTE_PRESENT (1UL << 0)
#define PTE_WRITE (1UL << 1)
#define PTE_USER (1UL << 2)

struct kvm_userspace_memory_region {
	unsigned int slot;
	unsigned int flags;
	unsigned long long guest_phys_addr;
	unsigned long long memory_size;
	unsigned long long userspace_addr;
};

struct kvm_regs {
	unsigned long long rax, rbx, rcx, rdx;
	unsigned long long rsi, rdi, rsp, rbp;
	unsigned long long r8, r9, r10, r11;
	unsigned long long r12, r13, r14, r15;
	unsigned long long rip, rflags;
};

struct kvm_segment {
	unsigned long long base;
	unsigned int limit;
	unsigned short selector;
	unsigned char type;
	unsigned char present, dpl, db, s, l, g, avl;
	unsigned char unusable;
	unsigned char padding;
};

struct kvm_dtable {
	unsigned long long base;
	unsigned short limit;
	unsigned short padding[3];
};

struct kvm_sregs {
	struct kvm_segment cs, ds, es, fs, gs, ss;
	struct kvm_segment tr, ldt;
	struct kvm_dtable gdt, idt;
	unsigned long long cr0, cr2, cr3, cr4, cr8;
	unsigned long long efer;
	unsigned long long apic_base;
	unsigned long long interrupt_bitmap[4];
};

struct kvm_msr_entry {
	unsigned int index;
	unsigned int reserved;
	unsigned long long data;
};

struct kvm_msrs_header {
	unsigned int nmsrs;
	unsigned int pad;
};

struct kvm_msrs {
	unsigned int nmsrs;
	unsigned int pad;
	struct kvm_msr_entry entries[4];
};

struct vcpu_setup {
	struct kvm_sregs sregs;
	struct kvm_regs regs;
	struct kvm_msrs msrs;
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
	union {
		struct {
			unsigned char direction;
			unsigned char size;
			unsigned short port;
			unsigned int count;
			unsigned long long data_offset;
		} io;
		unsigned char padding[256];
	};
};

struct sandbox_result {
	unsigned long long user_cs;
	unsigned long long kernel_cs;
	unsigned long long magic;
};

extern const unsigned char sandbox_guest_start[];
extern const unsigned char sandbox_guest_entry[];
extern const unsigned char sandbox_guest_syscall[];
extern const unsigned char sandbox_guest_user[];
extern const unsigned char sandbox_guest_attack[];
extern const unsigned char sandbox_guest_end[];

static unsigned char *guest_memory;

static long syscall3(long nr, long a0, long a1, long a2)
{
	long ret;
	__asm__ volatile("syscall"
			 : "=a"(ret)
			 : "a"(nr), "D"(a0), "S"(a1), "d"(a2)
			 : "rcx", "r11", "memory");
	return ret;
}

static long syscall6(long nr, long a0, long a1, long a2, long a3, long a4, long a5)
{
	register long r10 __asm__("r10") = a3;
	register long r8 __asm__("r8") = a4;
	register long r9 __asm__("r9") = a5;
	long ret;
	__asm__ volatile("syscall"
			 : "=a"(ret)
			 : "a"(nr), "D"(a0), "S"(a1), "d"(a2), "r"(r10), "r"(r8),
			   "r"(r9)
			 : "rcx", "r11", "memory");
	return ret;
}

static void syscall1_noreturn(long nr, long a0)
{
	__asm__ volatile("syscall" : : "a"(nr), "D"(a0) : "rcx", "r11", "memory");
	for (;;) {
	}
}

static long str_len(const char *s)
{
	long len = 0;
	while (s[len] != '\0')
		len++;
	return len;
}

static void puts(const char *s)
{
	syscall3(SYS_WRITE, 1, (long)s, str_len(s));
}

static void copy_bytes(void *dst, const void *src, unsigned long len)
{
	unsigned char *out = dst;
	const unsigned char *in = src;

	while (len--)
		*out++ = *in++;
}

static void zero_bytes(void *dst, unsigned long len)
{
	unsigned char *out = dst;

	while (len--)
		*out++ = 0;
}

static long sys_ioctl(long fd, unsigned long request, unsigned long arg)
{
	return syscall3(SYS_IOCTL, fd, request, arg);
}

static int fail(const char *message)
{
	puts(message);
	return 1;
}

static void set_segment(struct kvm_segment *segment, unsigned short selector, unsigned char type,
			unsigned char dpl, unsigned char long_mode)
{
	segment->base = 0;
	segment->limit = 0xffffffffU;
	segment->selector = selector;
	segment->type = type;
	segment->present = 1;
	segment->dpl = dpl;
	segment->db = long_mode ? 0 : 1;
	segment->s = 1;
	segment->l = long_mode;
	segment->g = 1;
	segment->avl = 0;
	segment->unusable = 0;
}

static void setup_guest_memory(unsigned long payload_gpa)
{
	unsigned long long *pml4 = (unsigned long long *)&guest_memory[PML4_GPA];
	unsigned long long *pdpt = (unsigned long long *)&guest_memory[PDPT_GPA];
	unsigned long long *pd = (unsigned long long *)&guest_memory[PD_GPA];
	unsigned long long *pt = (unsigned long long *)&guest_memory[PT_GPA];
	unsigned long long *gdt = (unsigned long long *)&guest_memory[GDT_GPA];
	unsigned long payload_page = payload_gpa & ~(PAGE_SIZE - 1);

	zero_bytes(guest_memory, GUEST_MEMORY_SIZE);
	pml4[0] = PDPT_GPA | PTE_PRESENT | PTE_WRITE | PTE_USER;
	pdpt[0] = PD_GPA | PTE_PRESENT | PTE_WRITE | PTE_USER;
	pd[0] = PT_GPA | PTE_PRESENT | PTE_WRITE | PTE_USER;
	for (unsigned long i = 0; i < GUEST_MEMORY_SIZE / PAGE_SIZE; i++) {
		unsigned long gpa = i * PAGE_SIZE;
		unsigned long flags = PTE_PRESENT | PTE_WRITE;
		if (gpa == payload_page || gpa == USER_STACK_TOP_GPA - PAGE_SIZE ||
		    gpa == RESULT_GPA)
			flags |= PTE_USER;
		pt[i] = gpa | flags;
	}

	gdt[0] = 0;
	gdt[1] = 0x00af9b000000ffffULL;
	gdt[2] = 0x00cf93000000ffffULL;
	gdt[3] = 0x00cff3000000ffffULL;
	gdt[4] = 0x00affb000000ffffULL;

	copy_bytes(&guest_memory[MONITOR_GPA], sandbox_guest_start,
		   (unsigned long)(sandbox_guest_end - sandbox_guest_start));
}

static int setup_vcpu(long vcpufd, struct vcpu_setup *setup, unsigned long entry_gpa,
		      unsigned long payload_gpa, unsigned long syscall_gpa)
{
	struct kvm_sregs *sregs = &setup->sregs;
	struct kvm_regs *regs = &setup->regs;
	struct kvm_msrs *msrs = &setup->msrs;

	zero_bytes(setup, sizeof(*setup));
	if (sys_ioctl(vcpufd, KVM_GET_SREGS, (unsigned long)sregs) != 0)
		return fail("KVM_GET_SREGS failed\n");

	set_segment(&sregs->cs, 0x08, 11, 0, 1);
	set_segment(&sregs->ds, 0x10, 3, 0, 0);
	sregs->es = sregs->ds;
	sregs->fs = sregs->ds;
	sregs->gs = sregs->ds;
	sregs->ss = sregs->ds;
	sregs->gdt.base = GDT_GPA;
	sregs->gdt.limit = 5 * sizeof(unsigned long long) - 1;
	sregs->idt.base = 0;
	sregs->idt.limit = 0;
	sregs->cr3 = PML4_GPA;
	sregs->cr4 = CR4_PAE;
	sregs->cr0 = CR0_PE | CR0_MP | CR0_ET | CR0_NE | CR0_WP | CR0_PG;
	sregs->efer = EFER_SCE | EFER_LME | EFER_LMA;
	if (sys_ioctl(vcpufd, KVM_SET_SREGS, (unsigned long)sregs) != 0)
		return fail("KVM_SET_SREGS failed\n");

	msrs->nmsrs = 3;
	msrs->entries[0].index = KVM_MSR_STAR;
	msrs->entries[0].data = (0x13ULL << 48) | (0x08ULL << 32);
	msrs->entries[1].index = KVM_MSR_LSTAR;
	msrs->entries[1].data = syscall_gpa;
	msrs->entries[2].index = KVM_MSR_SYSCALL_MASK;
	msrs->entries[2].data = 0;
	if (sys_ioctl(vcpufd, KVM_SET_MSRS, (unsigned long)msrs) != 3)
		return fail("KVM_SET_MSRS failed\n");

	regs->rip = entry_gpa;
	regs->rsp = 0x7000;
	regs->rflags = 0x2;
	regs->rdi = payload_gpa;
	regs->rsi = USER_STACK_TOP_GPA;
	if (sys_ioctl(vcpufd, KVM_SET_REGS, (unsigned long)regs) != 0)
		return fail("KVM_SET_REGS failed\n");
	return 0;
}

static int run_sandbox(void)
{
	unsigned long blob_size = (unsigned long)(sandbox_guest_end - sandbox_guest_start);
	unsigned long entry_gpa = MONITOR_GPA +
		(unsigned long)(sandbox_guest_entry - sandbox_guest_start);
	unsigned long syscall_gpa = MONITOR_GPA +
		(unsigned long)(sandbox_guest_syscall - sandbox_guest_start);
	unsigned long payload_gpa = MONITOR_GPA +
		(unsigned long)(sandbox_guest_user - sandbox_guest_start);
	struct sandbox_result *result;
	struct kvm_userspace_memory_region *region;
	struct vcpu_setup *setup;
	long kvmfd, vmfd, vcpufd, mmap_size;
	struct kvm_run *run;

	if (blob_size > 2 * PAGE_SIZE || (payload_gpa & (PAGE_SIZE - 1)) != 0)
		return fail("invalid sandbox guest layout\n");
	puts("kvm sandbox: start\n");

	kvmfd = syscall3(SYS_OPENAT, AT_FDCWD, (long)"/dev/kvm", O_RDWR | O_CLOEXEC);
	if (kvmfd < 0)
		return fail("open /dev/kvm failed\n");
	if (sys_ioctl(kvmfd, KVM_GET_API_VERSION, 0) != 12)
		return fail("KVM_GET_API_VERSION failed\n");
	mmap_size = sys_ioctl(kvmfd, KVM_GET_VCPU_MMAP_SIZE, 0);
	if (mmap_size < (long)sizeof(struct kvm_run))
		return fail("KVM_GET_VCPU_MMAP_SIZE failed\n");
	puts("kvm sandbox: KVM device ready\n");

	vmfd = sys_ioctl(kvmfd, KVM_CREATE_VM, 0);
	if (vmfd < 0)
		return fail("KVM_CREATE_VM failed\n");
	puts("kvm sandbox: VM created\n");

	guest_memory = (unsigned char *)syscall6(SYS_MMAP, 0, GUEST_MEMORY_SIZE,
						 PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS,
						 -1, 0);
	if ((long)guest_memory < 0)
		return fail("mmap guest memory failed\n");
	result = (struct sandbox_result *)&guest_memory[RESULT_GPA];

	setup_guest_memory(payload_gpa);
	puts("kvm sandbox: guest image ready\n");
	region = (struct kvm_userspace_memory_region *)guest_memory;
	zero_bytes(region, sizeof(*region));
	region->slot = 0;
	region->memory_size = GUEST_MEMORY_SIZE;
	region->userspace_addr = (unsigned long long)guest_memory;
	if (sys_ioctl(vmfd, KVM_SET_USER_MEMORY_REGION, (unsigned long)region) != 0)
		return fail("KVM_SET_USER_MEMORY_REGION failed\n");
	puts("kvm sandbox: guest memory registered\n");

	vcpufd = sys_ioctl(vmfd, KVM_CREATE_VCPU, 0);
	if (vcpufd < 0)
		return fail("KVM_CREATE_VCPU failed\n");
	run = (struct kvm_run *)syscall6(SYS_MMAP, 0, mmap_size, PROT_READ | PROT_WRITE,
					 MAP_SHARED, vcpufd, 0);
	if ((long)run < 0)
		return fail("mmap KVM vCPU failed\n");
	puts("kvm sandbox: vCPU ready\n");
	setup = (struct vcpu_setup *)guest_memory;
	if (setup_vcpu(vcpufd, setup, entry_gpa, payload_gpa, syscall_gpa) != 0)
		return 1;
	puts("kvm sandbox: guest ready\n");

	if (sys_ioctl(vcpufd, KVM_RUN, 0) != 0)
		return fail("KVM_RUN failed\n");
	puts("kvm sandbox: first exit\n");
	if (run->exit_reason != KVM_EXIT_IO || run->io.direction != KVM_EXIT_IO_OUT ||
	    run->io.port != 0xe9 || run->io.size != 1 || run->io.count != 1 ||
	    run->io.data_offset >= (unsigned long long)mmap_size ||
	    *((unsigned char *)run + run->io.data_offset) != 'P')
		return fail("sandbox monitor returned an unexpected KVM exit\n");

	if (result->user_cs != 0x23 || result->kernel_cs != 0x08 ||
	    result->magic != SANDBOX_MAGIC)
		return fail("sandbox privilege transition check failed\n");

	if (sys_ioctl(vcpufd, KVM_RUN, 0) != 0)
		return fail("second KVM_RUN failed\n");
	if (run->exit_reason != KVM_EXIT_SHUTDOWN)
		return fail("sandbox page isolation check failed\n");

	syscall3(SYS_CLOSE, vcpufd, 0, 0);
	syscall3(SYS_CLOSE, vmfd, 0, 0);
	syscall3(SYS_CLOSE, kvmfd, 0, 0);
	puts("kvm sandbox pass\n");
	return 0;
}

void _start(void)
{
	syscall1_noreturn(SYS_EXIT, run_sandbox());
}
