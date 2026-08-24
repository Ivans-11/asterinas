// SPDX-License-Identifier: MPL-2.0

#include "sandbox_abi.h"
#include "runner.h"
#include "elf_loader.h"
#include "syscall_proxy.h"

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
#define IOWR(type, nr, size)                                                                  \
	(((IOC_READ | IOC_WRITE) << IOC_DIRSHIFT) | ((size) << IOC_SIZESHIFT) |                 \
	 ((type) << IOC_TYPESHIFT) | (nr))

#define KVM_GET_API_VERSION IOC(KVMIO, 0x00)
#define KVM_CREATE_VM IOC(KVMIO, 0x01)
#define KVM_GET_VCPU_MMAP_SIZE IOC(KVMIO, 0x04)
#define KVM_CREATE_VCPU IOC(KVMIO, 0x41)
#define KVM_GET_SUPPORTED_CPUID IOWR(KVMIO, 0x05, sizeof(struct kvm_cpuid2_header))
#define KVM_SET_USER_MEMORY_REGION IOW(KVMIO, 0x46, sizeof(struct kvm_userspace_memory_region))
#define KVM_RUN IOC(KVMIO, 0x80)
#define KVM_SET_REGS IOW(KVMIO, 0x82, sizeof(struct kvm_regs))
#define KVM_GET_SREGS IOR(KVMIO, 0x83, sizeof(struct kvm_sregs))
#define KVM_SET_SREGS IOW(KVMIO, 0x84, sizeof(struct kvm_sregs))
#define KVM_SET_MSRS IOW(KVMIO, 0x89, sizeof(struct kvm_msrs_header))
#define KVM_SET_CPUID2 IOW(KVMIO, 0x90, sizeof(struct kvm_cpuid2_header))

#define KVM_EXIT_IO 2
#define KVM_EXIT_SHUTDOWN 8
#define KVM_EXIT_FAIL_ENTRY 9
#define KVM_EXIT_INTERNAL_ERROR 17
#define KVM_EXIT_IO_OUT 1

#define KVM_MSR_STAR 0xc0000081U
#define KVM_MSR_LSTAR 0xc0000082U
#define KVM_MSR_SYSCALL_MASK 0xc0000084U
#define EM_X86_64 62

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

#define PAGE_SIZE 0x1000UL
#define GUEST_MEMORY_SIZE 0x200000UL
#define PML4_GPA 0x1000UL
#define PDPT_GPA 0x2000UL
#define PD_GPA 0x3000UL
#define PT_GPA 0x4000UL
#define GDT_GPA 0x5000UL
#define TSS_GPA 0x6000UL
#define IDT_GPA 0x6800UL
#define MONITOR_STACK_TOP_GPA 0x7800UL
#define MONITOR_GPA 0x8000UL
#define APPLICATION_START_GPA 0x10000UL
#define USER_STACK_TOP_GPA 0x1fc000UL
#define RESULT_GPA 0xd000UL

#define CR0_PE (1UL << 0)
#define CR0_MP (1UL << 1)
#define CR0_ET (1UL << 4)
#define CR0_NE (1UL << 5)
#define CR0_WP (1UL << 16)
#define CR0_PG (1UL << 31)
#define CR4_PAE (1UL << 5)
#define CR4_OSFXSR (1UL << 9)
#define CR4_OSXMMEXCPT (1UL << 10)
#define EFER_SCE (1UL << 0)
#define EFER_LME (1UL << 8)
#define EFER_LMA (1UL << 10)
#define EFER_NXE (1UL << 11)
#define X86_ARCH_PRCTL 158
#define X86_ARCH_SET_FS 0x1002

#define PTE_PRESENT (1UL << 0)
#define PTE_WRITE (1UL << 1)
#define PTE_USER (1UL << 2)
#define PTE_NX (1ULL << 63)

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

struct kvm_cpuid_entry2 {
	unsigned int function;
	unsigned int index;
	unsigned int flags;
	unsigned int eax;
	unsigned int ebx;
	unsigned int ecx;
	unsigned int edx;
	unsigned int padding[3];
};

struct kvm_cpuid2_header {
	unsigned int nent;
	unsigned int padding;
};

struct kvm_cpuid2 {
	unsigned int nent;
	unsigned int padding;
	struct kvm_cpuid_entry2 entries[256];
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

extern const unsigned char sandbox_monitor_start[];
extern const unsigned char sandbox_monitor_entry[];
extern const unsigned char sandbox_monitor_syscall[];
extern const unsigned char sandbox_monitor_page_fault[];
extern const unsigned char sandbox_monitor_end[];

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

static int decode_exit(struct sandbox_runner *runner, unsigned int *event_kind)
{
	struct kvm_run *run = runner->run;
	unsigned char report;

	if (run->exit_reason != KVM_EXIT_IO || run->io.direction != KVM_EXIT_IO_OUT ||
	    run->io.port != 0xe9 || run->io.size != 1 || run->io.count != 1 ||
	    run->io.data_offset >= (unsigned long long)runner->run_mmap_size)
		return 1;
	report = *((unsigned char *)run + run->io.data_offset);
	if (report == SANDBOX_REPORT_SYSCALL)
		*event_kind = SANDBOX_EVENT_SYSCALL;
	else if (report == SANDBOX_REPORT_EXIT)
		*event_kind = SANDBOX_EVENT_EXIT;
	else if (report == SANDBOX_REPORT_FAULT)
		*event_kind = SANDBOX_EVENT_FAULT;
	else
		return 1;
	return 0;
}

static const struct sandbox_linux_abi linux_abi = { .write_number = 1,
	.read_number = 0, .clock_gettime_number = 228, .brk_number = 12,
	.openat_number = 257, .close_number = 3, .lseek_number = 8,
	.set_tid_address_number = 218, .set_robust_list_number = 273, .rseq_number = 334,
	.rt_sigprocmask_number = 14,
	.fstat_number = 5, .fstat_size = 144, .prlimit64_number = 302, .readlinkat_number = 267,
	.mprotect_number = 10, .riscv_hwprobe_number = ~0ULL,
	.getrandom_number = 318,
	.exit_number = 60, .exit_group_number = 231 };

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

static void setup_guest_memory(const struct sandbox_image *image)
{
	unsigned long long *pml4 = (unsigned long long *)&guest_memory[PML4_GPA];
	unsigned long long *pdpt = (unsigned long long *)&guest_memory[PDPT_GPA];
	unsigned long long *pd = (unsigned long long *)&guest_memory[PD_GPA];
	unsigned long long *pt = (unsigned long long *)&guest_memory[PT_GPA];
	unsigned long long *gdt = (unsigned long long *)&guest_memory[GDT_GPA];
	unsigned long long *idt = (unsigned long long *)&guest_memory[IDT_GPA];
	unsigned char *tss = &guest_memory[TSS_GPA];
	unsigned long page_fault_gpa = MONITOR_GPA +
		(unsigned long)(sandbox_monitor_page_fault - sandbox_monitor_start);

	pml4[0] = PDPT_GPA | PTE_PRESENT | PTE_WRITE | PTE_USER;
	pdpt[0] = PD_GPA | PTE_PRESENT | PTE_WRITE | PTE_USER;
	pd[0] = PT_GPA | PTE_PRESENT | PTE_WRITE | PTE_USER;
	for (unsigned long i = 0; i < GUEST_MEMORY_SIZE / PAGE_SIZE; i++) {
		unsigned long gpa = i * PAGE_SIZE;
		unsigned long long flags = PTE_PRESENT | PTE_WRITE | PTE_NX;

		if (gpa == MONITOR_GPA)
			flags = PTE_PRESENT;
		else if (i < sizeof(image->page_flags) && image->page_flags[i] != 0) {
			flags = PTE_PRESENT | PTE_USER;
			if (image->page_flags[i] & SANDBOX_IMAGE_PAGE_WRITE)
				flags |= PTE_WRITE | PTE_NX;
			else if (!(image->page_flags[i] & SANDBOX_IMAGE_PAGE_EXEC))
				flags |= PTE_NX;
		}
		else if (gpa >= image->image_end && gpa < USER_STACK_TOP_GPA - PAGE_SIZE &&
			 gpa != RESULT_GPA && gpa != SANDBOX_LAUNCH_ENTRY_GPA)
			flags = PTE_PRESENT | PTE_WRITE | PTE_USER | PTE_NX;
		else if (gpa == USER_STACK_TOP_GPA - PAGE_SIZE)
			flags = PTE_PRESENT | PTE_WRITE | PTE_USER | PTE_NX;
		pt[i] = gpa | flags;
	}

	gdt[0] = 0;
	gdt[1] = 0x00af9b000000ffffULL;
	gdt[2] = 0x00cf93000000ffffULL;
	gdt[3] = 0x00cff3000000ffffULL;
	gdt[4] = 0x00affb000000ffffULL;
	gdt[5] = 0x0000890060000067ULL;
	gdt[6] = 0;

	*(unsigned long long *)&tss[4] = MONITOR_STACK_TOP_GPA;
	*(unsigned short *)&tss[102] = 104;
	idt[28] = (page_fault_gpa & 0xffffULL) | (0x08ULL << 16) | (0x8eULL << 40) |
		  ((page_fault_gpa & 0xffff0000ULL) << 32);
	idt[29] = page_fault_gpa >> 32;

}

static int setup_vcpu(long vcpufd, struct vcpu_setup *setup, unsigned long entry_gpa,
		      unsigned long payload_gpa, unsigned long stack_gpa,
		      unsigned long syscall_gpa)
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
	sregs->gdt.limit = 7 * sizeof(unsigned long long) - 1;
	sregs->idt.base = IDT_GPA;
	sregs->idt.limit = 15 * 16 - 1;
	sregs->tr.base = TSS_GPA;
	sregs->tr.limit = 103;
	sregs->tr.selector = 0x28;
	sregs->tr.type = 11;
	sregs->tr.present = 1;
	sregs->tr.dpl = 0;
	sregs->tr.s = 0;
	sregs->tr.unusable = 0;
	sregs->cr3 = PML4_GPA;
	sregs->cr4 = CR4_PAE | CR4_OSFXSR | CR4_OSXMMEXCPT;
	sregs->cr0 = CR0_PE | CR0_MP | CR0_ET | CR0_NE | CR0_WP | CR0_PG;
	sregs->efer = EFER_SCE | EFER_LME | EFER_LMA | EFER_NXE;
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
	regs->rsp = MONITOR_STACK_TOP_GPA;
	regs->rflags = 0x2;
	regs->rdi = payload_gpa;
	regs->rsi = stack_gpa;
	if (sys_ioctl(vcpufd, KVM_SET_REGS, (unsigned long)regs) != 0)
		return fail("KVM_SET_REGS failed\n");
	return 0;
}

static int setup_cpuid(long kvmfd, long vcpufd)
{
	static struct kvm_cpuid2 cpuid;

	cpuid.nent = 256;
	if (sys_ioctl(kvmfd, KVM_GET_SUPPORTED_CPUID, (unsigned long)&cpuid) != 0)
		return fail("KVM_GET_SUPPORTED_CPUID failed\n");
	for (unsigned int index = 0; index < cpuid.nent; index++) {
		struct kvm_cpuid_entry2 *entry = &cpuid.entries[index];

		if (entry->function == 1 && entry->index == 0)
			entry->ecx &= ~((1U << 26) | (1U << 27) | (1U << 28));
	}
	if (sys_ioctl(vcpufd, KVM_SET_CPUID2, (unsigned long)&cpuid) != 0)
		return fail("KVM_SET_CPUID2 failed\n");
	return 0;
}

static int prepare_backend(struct sandbox_runner *runner,
			   const struct sandbox_image *image,
			   unsigned long long stack_pointer)
{
	unsigned long blob_size = (unsigned long)(sandbox_monitor_end - sandbox_monitor_start);
	unsigned long entry_gpa = MONITOR_GPA +
		(unsigned long)(sandbox_monitor_entry - sandbox_monitor_start);
	unsigned long syscall_gpa = MONITOR_GPA +
		(unsigned long)(sandbox_monitor_syscall - sandbox_monitor_start);
	struct vcpu_setup *setup;

	if (blob_size > PAGE_SIZE)
		return fail("invalid sandbox guest layout\n");
	guest_memory = runner->guest_memory;
	copy_bytes(&guest_memory[MONITOR_GPA], sandbox_monitor_start, blob_size);
	setup_guest_memory(image);
	if (setup_cpuid(runner->kvm_fd, runner->vcpu_fd) != 0)
		return 1;
	setup = (struct vcpu_setup *)guest_memory;
	return setup_vcpu(runner->vcpu_fd, setup, entry_gpa, image->entry,
			  stack_pointer, syscall_gpa);
}

static int service_event(struct sandbox_runner *runner,
			 volatile struct sandbox_control *control,
			 const struct sandbox_event *event)
{
	struct kvm_sregs sregs;
	unsigned long long base;

	if (event->kind != SANDBOX_EVENT_SYSCALL || event->number != X86_ARCH_PRCTL)
		return 1;
	base = event->args[1];
	if (event->args[0] != X86_ARCH_SET_FS || base >= runner->guest_memory_size) {
		fail("sandbox arch_prctl arguments are invalid\n");
		return -1;
	}
	if (sys_ioctl(runner->vcpu_fd, KVM_GET_SREGS, (unsigned long)&sregs) != 0) {
		fail("sandbox KVM_GET_SREGS failed\n");
		return -1;
	}
	sregs.fs.base = base;
	if (sys_ioctl(runner->vcpu_fd, KVM_SET_SREGS, (unsigned long)&sregs) != 0) {
		fail("sandbox KVM_SET_SREGS failed\n");
		return -1;
	}
	control->result = 0;
	control->error = 0;
	control->action = SANDBOX_ACTION_RESUME;
	return 0;
}

const struct sandbox_backend_ops sandbox_backend = {
	.guest_memory_size = GUEST_MEMORY_SIZE,
	.application_start = APPLICATION_START_GPA,
	.stack_bottom = USER_STACK_TOP_GPA - SANDBOX_USER_STACK_SIZE,
	.stack_top = USER_STACK_TOP_GPA,
	.control_gpa = RESULT_GPA,
	.elf_machine = EM_X86_64,
	.linux_abi = &linux_abi,
	.prepare = prepare_backend,
	.decode_exit = decode_exit,
	.service_event = service_event,
};
