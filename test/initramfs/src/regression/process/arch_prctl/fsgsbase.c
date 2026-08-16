// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <stdint.h>
#include <asm/prctl.h>
#include <cpuid.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

#define CPUID_FSGSBASE (1U << 0)

static unsigned long gs_slots[2];

static int cpu_has_fsgsbase(void)
{
	unsigned int eax;
	unsigned int ebx;
	unsigned int ecx;
	unsigned int edx;

	if (!__get_cpuid_count(7, 0, &eax, &ebx, &ecx, &edx)) {
		return 0;
	}

	return ebx & CPUID_FSGSBASE;
}

static int arch_get_gs(uintptr_t *gs_base)
{
	return syscall(SYS_arch_prctl, ARCH_GET_GS, gs_base);
}

static int arch_get_fs(uintptr_t *fs_base)
{
	return syscall(SYS_arch_prctl, ARCH_GET_FS, fs_base);
}

static int arch_set_gs(uintptr_t gs_base)
{
	return syscall(SYS_arch_prctl, ARCH_SET_GS, gs_base);
}

static uintptr_t read_gsbase(void)
{
	uintptr_t gs_base;

	asm volatile("rdgsbase %0" : "=r"(gs_base) : : "memory");
	return gs_base;
}

static uintptr_t read_fsbase(void)
{
	uintptr_t fs_base;

	asm volatile("rdfsbase %0" : "=r"(fs_base) : : "memory");
	return fs_base;
}

static void write_gsbase(uintptr_t gs_base)
{
	asm volatile("wrgsbase %0" : : "r"(gs_base) : "memory");
}

FN_TEST(sync_fsgsbase_from_cpu)
{
	SKIP_TEST_IF(!cpu_has_fsgsbase());

	uintptr_t reported_fs = 0;
	TEST_SUCC(arch_get_fs(&reported_fs));
	TEST_RES(reported_fs, _ret == read_fsbase());

	uintptr_t syscall_gs = (uintptr_t)&gs_slots[0];
	TEST_SUCC(arch_set_gs(syscall_gs));
	usleep(100); // Trigger a syscall and context switch.
	TEST_RES(read_gsbase(), _ret == syscall_gs);
	uintptr_t reported_gs = 0;
	TEST_SUCC(arch_get_gs(&reported_gs));
	TEST_RES(reported_gs, _ret == syscall_gs);

	uintptr_t fsgsbase_gs = (uintptr_t)&gs_slots[1];
	write_gsbase(fsgsbase_gs);
	usleep(100); // Trigger a syscall and context switch.
	TEST_RES(read_gsbase(), _ret == fsgsbase_gs);
	reported_gs = 0;
	TEST_SUCC(arch_get_gs(&reported_gs));
	TEST_RES(reported_gs, _ret == fsgsbase_gs);
}
END_TEST()
