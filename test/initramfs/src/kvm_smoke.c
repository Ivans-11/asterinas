// SPDX-License-Identifier: MPL-2.0

#define KVMIO 0xae
#define IOC(type, nr) (((type) << 8) | (nr))
#define IOC_WRITE 1UL
#define IOC_TYPESHIFT 8
#define IOC_SIZESHIFT 16
#define IOC_DIRSHIFT 30
#define IOW(type, nr, size)                                                                    \
	((IOC_WRITE << IOC_DIRSHIFT) | ((size) << IOC_SIZESHIFT) | ((type) << IOC_TYPESHIFT) |  \
	 (nr))
#define KVM_GET_API_VERSION IOC(KVMIO, 0x00)
#define KVM_CREATE_VM IOC(KVMIO, 0x01)
#define KVM_CHECK_EXTENSION IOC(KVMIO, 0x03)
#define KVM_GET_VCPU_MMAP_SIZE IOC(KVMIO, 0x04)
#define KVM_CREATE_VCPU IOC(KVMIO, 0x41)
#define KVM_SET_USER_MEMORY_REGION IOW(KVMIO, 0x46, sizeof(struct kvm_userspace_memory_region))

#define KVM_CAP_USER_MEMORY 3
#define KVM_CAP_NR_VCPUS 9
#define KVM_CAP_NR_MEMSLOTS 10
#define KVM_CAP_MAX_VCPUS 66
#define KVM_CAP_IMMEDIATE_EXIT 136

#define AT_FDCWD -100
#define ENOTTY 25
#define O_RDWR 02
#define O_CLOEXEC 02000000

struct kvm_userspace_memory_region {
	unsigned int slot;
	unsigned int flags;
	unsigned long long guest_phys_addr;
	unsigned long long memory_size;
	unsigned long long userspace_addr;
};

static unsigned char guest_memory[4096] __attribute__((aligned(4096)));

#if defined(__riscv) && __riscv_xlen == 64
#define SYS_OPENAT 56
#define SYS_CLOSE 57
#define SYS_IOCTL 29
#define SYS_WRITE 64
#define SYS_EXIT 93

static long syscall3(long nr, long a0, long a1, long a2)
{
	register long x10 __asm__("a0") = a0;
	register long x11 __asm__("a1") = a1;
	register long x12 __asm__("a2") = a2;
	register long x17 __asm__("a7") = nr;
	__asm__ volatile("ecall" : "+r"(x10) : "r"(x11), "r"(x12), "r"(x17) : "memory");
	return x10;
}

static void syscall1_noreturn(long nr, long a0)
{
	register long x10 __asm__("a0") = a0;
	register long x17 __asm__("a7") = nr;
	__asm__ volatile("ecall" : : "r"(x10), "r"(x17) : "memory");
	for (;;) {
	}
}
#elif defined(__x86_64__)
#define SYS_WRITE 1
#define SYS_CLOSE 3
#define SYS_IOCTL 16
#define SYS_OPENAT 257
#define SYS_EXIT 60

static long syscall3(long nr, long a0, long a1, long a2)
{
	register long r10 __asm__("r10") = a2;
	long ret;
	__asm__ volatile("syscall"
			 : "=a"(ret)
			 : "a"(nr), "D"(a0), "S"(a1), "d"(a2), "r"(r10)
			 : "rcx", "r11", "memory");
	return ret;
}

static void syscall1_noreturn(long nr, long a0)
{
	__asm__ volatile("syscall" : : "a"(nr), "D"(a0) : "rcx", "r11", "memory");
	for (;;) {
	}
}
#else
#error "unsupported architecture"
#endif

static long sys_openat(long dirfd, const char *path, long flags)
{
	return syscall3(SYS_OPENAT, dirfd, (long)path, flags);
}

static long sys_ioctl(long fd, long request, long arg)
{
	return syscall3(SYS_IOCTL, fd, request, arg);
}

static long sys_write(long fd, const char *buf, long len)
{
	return syscall3(SYS_WRITE, fd, (long)buf, len);
}

static long sys_close(long fd)
{
	return syscall3(SYS_CLOSE, fd, 0, 0);
}

static void sys_exit(long code)
{
	syscall1_noreturn(SYS_EXIT, code);
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
	sys_write(1, s, str_len(s));
}

static int expect_ioctl(long fd, unsigned long request, unsigned long arg, long expected,
			const char *name)
{
	long value = sys_ioctl(fd, request, arg);
	if (value != expected) {
		puts(name);
		puts(": unexpected value\n");
		return 1;
	}
	return 0;
}

static int expect_ioctl_errno(long fd, unsigned long request, unsigned long arg, long expected_errno,
			      const char *name)
{
	long value = sys_ioctl(fd, request, arg);
	if (value != -expected_errno) {
		puts(name);
		puts(": unexpected errno\n");
		return 1;
	}
	return 0;
}

static int main(void)
{
	long fd = sys_openat(AT_FDCWD, "/dev/kvm", O_RDWR | O_CLOEXEC);
	if (fd < 0) {
		puts("open /dev/kvm failed\n");
		return 1;
	}

	if (expect_ioctl(fd, KVM_GET_API_VERSION, 0, 12, "KVM_GET_API_VERSION") != 0)
		return 1;
	if (expect_ioctl(fd, KVM_CHECK_EXTENSION, KVM_CAP_USER_MEMORY, 1,
			 "KVM_CAP_USER_MEMORY") != 0)
		return 1;
	if (expect_ioctl(fd, KVM_CHECK_EXTENSION, KVM_CAP_NR_VCPUS, 1, "KVM_CAP_NR_VCPUS") !=
	    0)
		return 1;
	if (expect_ioctl(fd, KVM_CHECK_EXTENSION, KVM_CAP_MAX_VCPUS, 1,
			 "KVM_CAP_MAX_VCPUS") != 0)
		return 1;
	if (expect_ioctl(fd, KVM_CHECK_EXTENSION, KVM_CAP_NR_MEMSLOTS, 32,
			 "KVM_CAP_NR_MEMSLOTS") != 0)
		return 1;
	if (expect_ioctl(fd, KVM_CHECK_EXTENSION, KVM_CAP_IMMEDIATE_EXIT, 1,
			 "KVM_CAP_IMMEDIATE_EXIT") != 0)
		return 1;
	if (expect_ioctl(fd, KVM_GET_VCPU_MMAP_SIZE, 0, 0x1000, "KVM_GET_VCPU_MMAP_SIZE") !=
	    0)
		return 1;

	long vmfd = sys_ioctl(fd, KVM_CREATE_VM, 0);
	if (vmfd < 0) {
		puts("KVM_CREATE_VM failed\n");
		return 1;
	}
	if (expect_ioctl_errno(vmfd, KVM_GET_API_VERSION, 0, ENOTTY,
			       "VM fd KVM_GET_API_VERSION") != 0)
		return 1;

	struct kvm_userspace_memory_region memory_region = {
		.slot = 0,
		.flags = 0,
		.guest_phys_addr = 0x100000,
		.memory_size = sizeof(guest_memory),
		.userspace_addr = (unsigned long long)guest_memory,
	};
	if (expect_ioctl(vmfd, KVM_SET_USER_MEMORY_REGION, (long)&memory_region, 0,
			 "KVM_SET_USER_MEMORY_REGION") != 0)
		return 1;

	long vcpufd = sys_ioctl(vmfd, KVM_CREATE_VCPU, 0);
	if (vcpufd < 0) {
		puts("KVM_CREATE_VCPU failed\n");
		return 1;
	}
	sys_close(vcpufd);

	sys_close(vmfd);

	sys_close(fd);
	puts("kvm smoke pass\n");
	return 0;
}

void _start(void)
{
	sys_exit(main());
}
