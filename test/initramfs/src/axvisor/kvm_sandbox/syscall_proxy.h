/* SPDX-License-Identifier: MPL-2.0 */
#ifndef AXVISOR_KVM_SANDBOX_SYSCALL_PROXY_H
#define AXVISOR_KVM_SANDBOX_SYSCALL_PROXY_H

#include "runner.h"

struct sandbox_linux_abi {
	unsigned long long write_number;
	unsigned long long read_number;
	unsigned long long clock_gettime_number;
	unsigned long long brk_number;
	unsigned long long openat_number;
	unsigned long long close_number;
	unsigned long long lseek_number;
	unsigned long long set_tid_address_number;
	unsigned long long set_robust_list_number;
	unsigned long long rseq_number;
	unsigned long long rt_sigprocmask_number;
	unsigned long long fstat_number;
	unsigned long fstat_size;
	unsigned long long prlimit64_number;
	unsigned long long readlinkat_number;
	unsigned long long mprotect_number;
	unsigned long long riscv_hwprobe_number;
	unsigned long long getrandom_number;
	unsigned long long exit_number;
	unsigned long long exit_group_number;
};

int sandbox_service_linux_event(struct sandbox_runner *runner,
				volatile struct sandbox_control *control,
				const struct sandbox_event *event,
				const struct sandbox_backend_ops *backend);

#endif
