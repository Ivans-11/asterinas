// SPDX-License-Identifier: MPL-2.0

#include "runner.h"
#include "elf_loader.h"

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

#define KVM_EXIT_FAIL_ENTRY 9
#define KVM_EXIT_INTERNAL_ERROR 17

struct kvm_run_header {
	unsigned char request_interrupt_window;
	unsigned char immediate_exit;
	unsigned char padding[6];
	unsigned int exit_reason;
};

#define AT_FDCWD -100
#define MAP_PRIVATE 0x02
#define MAP_SHARED 0x01
#define MAP_ANONYMOUS 0x20
#define O_RDWR 02
#define O_CLOEXEC 02000000
#define PROT_READ 0x1
#define PROT_WRITE 0x2

#if defined(__x86_64__)
#define SYS_WRITE 1
#define SYS_READ 0
#define SYS_CLOSE 3
#define SYS_IOCTL 16
#define SYS_MMAP 9
#define SYS_MUNMAP 11
#define SYS_OPENAT 257
#define SYS_LSEEK 8
#define SYS_FSTAT 5
#define SYS_CLOCK_GETTIME 228
#define SYS_FORK 57
#define SYS_WAIT4 61
#define SYS_NANOSLEEP 35

static long syscall1(long number, long argument0)
{
	long result;
	__asm__ volatile("syscall"
			 : "=a"(result)
			 : "a"(number), "D"(argument0)
			 : "rcx", "r11", "memory");
	return result;
}

static long syscall3(long number, long argument0, long argument1, long argument2)
{
	long result;
	__asm__ volatile("syscall"
			 : "=a"(result)
			 : "a"(number), "D"(argument0), "S"(argument1), "d"(argument2)
			 : "rcx", "r11", "memory");
	return result;
}

static long syscall2(long number, long argument0, long argument1)
{
	long result;
	__asm__ volatile("syscall"
			 : "=a"(result)
			 : "a"(number), "D"(argument0), "S"(argument1)
			 : "rcx", "r11", "memory");
	return result;
}

static long syscall6(long number, long argument0, long argument1, long argument2, long argument3,
		     long argument4, long argument5)
{
	register long r10 __asm__("r10") = argument3;
	register long r8 __asm__("r8") = argument4;
	register long r9 __asm__("r9") = argument5;
	long result;

	__asm__ volatile("syscall"
			 : "=a"(result)
			 : "a"(number), "D"(argument0), "S"(argument1), "d"(argument2),
			   "r"(r10), "r"(r8), "r"(r9)
			 : "rcx", "r11", "memory");
	return result;
}
#elif defined(__riscv)
#define SYS_CLOSE 57
#define SYS_IOCTL 29
#define SYS_MMAP 222
#define SYS_MUNMAP 215
#define SYS_OPENAT 56
#define SYS_LSEEK 62
#define SYS_FSTAT 80
#define SYS_CLOCK_GETTIME 113
#define SYS_CLONE 220
#define SYS_WAIT4 260
#define SYS_NANOSLEEP 101
#define SYS_WRITE 64
#define SYS_READ 63

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

static long syscall2(long number, long argument0, long argument1)
{
	register long a0 __asm__("a0") = argument0;
	register long a1 __asm__("a1") = argument1;
	register long a7 __asm__("a7") = number;
	__asm__ volatile("ecall" : "+r"(a0) : "r"(a1), "r"(a7) : "memory");
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
#else
#error unsupported sandbox host architecture
#endif

long sandbox_ioctl(long fd, unsigned long request, unsigned long argument)
{
	return syscall3(SYS_IOCTL, fd, request, argument);
}

long sandbox_open(const char *path)
{
	return syscall3(SYS_OPENAT, AT_FDCWD, (long)path, O_CLOEXEC);
}

long sandbox_read(long fd, void *buffer, unsigned long length)
{
	return syscall3(SYS_READ, fd, (long)buffer, length);
}

long sandbox_write(long fd, const void *buffer, unsigned long length)
{
	return syscall3(SYS_WRITE, fd, (long)buffer, length);
}

long sandbox_seek(long fd, unsigned long long offset)
{
	return syscall3(SYS_LSEEK, fd, (long)offset, 0);
}

long sandbox_fstat(long fd, void *stat_buffer)
{
	return syscall2(SYS_FSTAT, fd, (long)stat_buffer);
}

long sandbox_clock_gettime(long clock_id, void *timespec)
{
	return syscall2(SYS_CLOCK_GETTIME, clock_id, (long)timespec);
}

long sandbox_fork(void)
{
#if defined(__x86_64__)
	return syscall1(SYS_FORK, 0);
#else
	/* SIGCHLD is the only flag required for fork-like clone semantics. */
	return syscall6(SYS_CLONE, 17, 0, 0, 0, 0, 0);
#endif
}

long sandbox_wait(long pid, int *status, int nohang)
{
	return syscall6(SYS_WAIT4, pid, (long)status, nohang ? 1 : 0, 0, 0, 0);
}

int sandbox_sleep_milliseconds(unsigned long milliseconds)
{
	struct {
		long seconds;
		long nanoseconds;
	} duration;

	duration.seconds = milliseconds / 1000;
	duration.nanoseconds = (long)(milliseconds % 1000) * 1000000;
	return syscall2(SYS_NANOSLEEP, (long)&duration, 0) == 0 ? 0 : 1;
}

long sandbox_close(long fd)
{
	return syscall1(SYS_CLOSE, fd);
}

long sandbox_unmap(void *address, unsigned long length)
{
	return syscall2(SYS_MUNMAP, (long)address, length);
}

void sandbox_print(const char *message)
{
	unsigned long length = 0;
	while (message[length] != '\0')
		length++;
	syscall3(SYS_WRITE, 1, (long)message, length);
}

int sandbox_fail(const char *message)
{
	sandbox_print(message);
	return 1;
}

void sandbox_copy(void *destination, const void *source, unsigned long length)
{
	unsigned char *output = destination;
	const unsigned char *input = source;
	while (length--)
		*output++ = *input++;
}

void sandbox_zero(void *destination, unsigned long length)
{
	unsigned char *output = destination;
	while (length--)
		*output++ = 0;
}

int sandbox_prepare_initial_stack(struct sandbox_runner *runner, long argument_count,
				  char *const arguments[],
				  const struct sandbox_image *image,
				  unsigned long long stack_bottom,
				  unsigned long long stack_top,
				  unsigned long long *stack_pointer)
{
	static const unsigned char random_seed[16] = {
		0x42, 0x17, 0xa9, 0x6c, 0x35, 0xd2, 0x8e, 0xf1,
		0x73, 0x0b, 0xc4, 0x59, 0x86, 0xed, 0x21, 0x9a,
	};
	unsigned long long guest_arguments[SANDBOX_MAX_ARGUMENTS];
	unsigned long long random_address;
	unsigned long word_count;
	unsigned long length;
	long argument_index;
	unsigned long long cursor;
	unsigned long long *words;
	unsigned long index;

	if (argument_count <= 0 || argument_count > SANDBOX_MAX_ARGUMENTS ||
	    stack_bottom >= stack_top || stack_top > runner->guest_memory_size)
		return sandbox_fail("sandbox initial stack is invalid\n");

	cursor = stack_top;
	for (argument_index = argument_count - 1; argument_index >= 0; argument_index--) {
		length = 0;
		while (arguments[argument_index][length] != '\0') {
			if (length >= 255)
				return sandbox_fail("sandbox application argument is too long\n");
			length++;
		}
		if (length + 1 > cursor - stack_bottom)
			return sandbox_fail("sandbox arguments exceed initial stack\n");
		cursor -= length + 1;
		sandbox_copy(&runner->guest_memory[cursor], arguments[argument_index], length + 1);
		guest_arguments[argument_index] = cursor;
	}
	if (sizeof(random_seed) > cursor - stack_bottom)
		return sandbox_fail("sandbox arguments exceed initial stack\n");
	cursor -= sizeof(random_seed);
	sandbox_copy(&runner->guest_memory[cursor], random_seed, sizeof(random_seed));
	random_address = cursor;

	cursor &= ~0xfULL;
	word_count = (unsigned long)argument_count + 17;
	if (cursor < stack_bottom + word_count * sizeof(unsigned long long))
		return sandbox_fail("sandbox initial stack is too small\n");
	cursor -= word_count * sizeof(unsigned long long);
	cursor &= ~0xfULL;
	words = (unsigned long long *)&runner->guest_memory[cursor];
	index = 0;
	words[index++] = (unsigned long long)argument_count;
	for (argument_index = 0; argument_index < argument_count; argument_index++)
		words[index++] = guest_arguments[argument_index];
	words[index++] = 0;
	words[index++] = 0;
	words[index++] = 6;
	words[index++] = 4096;
	words[index++] = 9;
	words[index++] = image->entry;
	words[index++] = 3;
	words[index++] = image->program_header_address;
	words[index++] = 4;
	words[index++] = image->program_header_entry_size;
	words[index++] = 5;
	words[index++] = image->program_header_count;
	words[index++] = 25;
	words[index++] = random_address;
	words[index++] = 0;
	words[index] = 0;
	*stack_pointer = cursor;
	return 0;
}

static int runner_create_failure(struct sandbox_runner *runner, const char *message)
{
	sandbox_fail(message);
	sandbox_runner_destroy(runner);
	return 1;
}

int sandbox_runner_create(struct sandbox_runner *runner, unsigned long guest_memory_size)
{
	struct kvm_userspace_memory_region region;
	unsigned int fd_index;

	sandbox_zero(runner, sizeof(*runner));
	runner->kvm_fd = runner->vm_fd = runner->vcpu_fd = -1;
	runner->guest_memory_size = guest_memory_size;
	runner->exit_limit = SANDBOX_DEFAULT_EVENT_LIMIT;
	runner->io_limit = SANDBOX_DEFAULT_IO_LIMIT;
	for (fd_index = 0; fd_index < SANDBOX_MAX_GUEST_FDS; fd_index++)
		runner->guest_host_fds[fd_index] = -1;
	runner->guest_host_fds[0] = 0;
	runner->guest_host_fds[1] = 1;
	runner->guest_host_fds[2] = 2;
	runner->kvm_fd = syscall3(SYS_OPENAT, AT_FDCWD, (long)"/dev/kvm", O_RDWR | O_CLOEXEC);
	if (runner->kvm_fd < 0)
		return runner_create_failure(runner, "open /dev/kvm failed\n");
	if (sandbox_ioctl(runner->kvm_fd, KVM_GET_API_VERSION, 0) != SANDBOX_KVM_API_VERSION)
		return runner_create_failure(runner, "KVM_GET_API_VERSION failed\n");
	runner->run_mmap_size = sandbox_ioctl(runner->kvm_fd, KVM_GET_VCPU_MMAP_SIZE, 0);
	if (runner->run_mmap_size < 32)
		return runner_create_failure(runner, "KVM_GET_VCPU_MMAP_SIZE failed\n");

	runner->vm_fd = sandbox_ioctl(runner->kvm_fd, KVM_CREATE_VM, 0);
	if (runner->vm_fd < 0)
		return runner_create_failure(runner, "KVM_CREATE_VM failed\n");
	runner->guest_memory = (unsigned char *)syscall6(
		SYS_MMAP, 0, guest_memory_size, PROT_READ | PROT_WRITE,
		MAP_SHARED | MAP_ANONYMOUS, -1, 0);
	if ((long)runner->guest_memory < 0)
		return runner_create_failure(runner, "mmap guest memory failed\n");

	sandbox_zero(&region, sizeof(region));
	region.slot = 0;
	region.memory_size = guest_memory_size;
	region.userspace_addr = (unsigned long long)runner->guest_memory;
	if (sandbox_ioctl(runner->vm_fd, KVM_SET_USER_MEMORY_REGION,
			  (unsigned long)&region) != 0)
		return runner_create_failure(runner, "KVM_SET_USER_MEMORY_REGION failed\n");
	runner->memory_registered = 1;

	runner->vcpu_fd = sandbox_ioctl(runner->vm_fd, KVM_CREATE_VCPU, 0);
	if (runner->vcpu_fd < 0)
		return runner_create_failure(runner, "KVM_CREATE_VCPU failed\n");
	runner->run = (void *)syscall6(SYS_MMAP, 0, runner->run_mmap_size,
				       PROT_READ | PROT_WRITE, MAP_SHARED, runner->vcpu_fd, 0);
	if ((long)runner->run < 0)
		return runner_create_failure(runner, "mmap KVM vCPU failed\n");
	return 0;
}

void sandbox_runner_destroy(struct sandbox_runner *runner)
{
	struct kvm_userspace_memory_region region;
	unsigned int fd_index;

	for (fd_index = 3; fd_index < SANDBOX_MAX_GUEST_FDS; fd_index++) {
		if (runner->guest_host_fds[fd_index] >= 0) {
			sandbox_close(runner->guest_host_fds[fd_index]);
			runner->guest_host_fds[fd_index] = -1;
		}
	}

	if (runner->run && (long)runner->run >= 0)
		syscall2(SYS_MUNMAP, (long)runner->run, runner->run_mmap_size);
	if (runner->memory_registered && runner->vm_fd >= 0) {
		sandbox_zero(&region, sizeof(region));
		region.slot = 0;
		sandbox_ioctl(runner->vm_fd, KVM_SET_USER_MEMORY_REGION,
			      (unsigned long)&region);
	}
	if (runner->vcpu_fd >= 0)
		syscall1(SYS_CLOSE, runner->vcpu_fd);
	if (runner->vm_fd >= 0)
		syscall1(SYS_CLOSE, runner->vm_fd);
	if (runner->guest_memory && (long)runner->guest_memory >= 0)
		syscall2(SYS_MUNMAP, (long)runner->guest_memory, runner->guest_memory_size);
	if (runner->kvm_fd >= 0)
		syscall1(SYS_CLOSE, runner->kvm_fd);
}

int sandbox_runner_abort(struct sandbox_runner *runner, const char *message)
{
	sandbox_fail(message);
	sandbox_runner_destroy(runner);
	return 1;
}

void sandbox_runner_request_exit(struct sandbox_runner *runner)
{
	struct kvm_run_header *run = runner->run;

	if (run && (long)run >= 0)
		run->immediate_exit = 1;
}

int sandbox_runner_exit_requested(const struct sandbox_runner *runner)
{
	const volatile struct kvm_run_header *run = runner->run;

	return run && (long)run >= 0 && run->immediate_exit != 0;
}

int sandbox_runner_next_event(struct sandbox_runner *runner,
			      const struct sandbox_backend_ops *backend,
			      volatile const struct sandbox_control *control,
			      struct sandbox_event *event)
{
	struct kvm_run_header *run = runner->run;
	unsigned int reported_event;

	if (runner->exit_count >= runner->exit_limit)
		return sandbox_fail("sandbox guest exceeded exit budget\n");
	if (sandbox_ioctl(runner->vcpu_fd, KVM_RUN, 0) != 0)
		return sandbox_fail("KVM_RUN failed\n");
	if (run->exit_reason == KVM_EXIT_FAIL_ENTRY ||
	    run->exit_reason == KVM_EXIT_INTERNAL_ERROR)
		return sandbox_fail("sandbox guest entered an invalid KVM state\n");
	if (control->magic != SANDBOX_MAGIC || control->version != SANDBOX_ABI_VERSION)
		return sandbox_fail("sandbox guest used an invalid control ABI\n");
	reported_event = control->event;
	if (backend->decode_exit(runner, &reported_event) != 0)
		return sandbox_fail("sandbox backend returned an unexpected KVM exit\n");
	if (reported_event != control->event)
		return sandbox_fail("sandbox control event does not match KVM report\n");
	if (control->sequence != (unsigned long long)runner->exit_count + 1)
		return sandbox_fail("sandbox guest used an invalid event sequence\n");

	event->kind = reported_event;
	event->number = control->number;
	for (unsigned int index = 0; index < 6; index++)
		event->args[index] = control->args[index];
	runner->exit_count++;
	return 0;
}
