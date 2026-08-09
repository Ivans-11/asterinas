/* SPDX-License-Identifier: MPL-2.0 */
#ifndef AXVISOR_KVM_SANDBOX_ABI_H
#define AXVISOR_KVM_SANDBOX_ABI_H

/* Stable values shared by a sandbox monitor and its guest payload. */
#define SANDBOX_ABI_VERSION 1
#define SANDBOX_MAGIC_VALUE 0x53414e44424f5821
#define SANDBOX_EVENT_PRIVILEGE 1
#define SANDBOX_EVENT_FAULT 2
#define SANDBOX_EVENT_SYSCALL 3
#define SANDBOX_EVENT_EXIT 4
#define SANDBOX_EVENT_CONSOLE 5

#define SANDBOX_ACTION_RESUME 0
#define SANDBOX_ACTION_TERMINATE 1

#define SANDBOX_CONTROL_MAGIC_OFFSET 0
#define SANDBOX_CONTROL_VERSION_OFFSET 8
#define SANDBOX_CONTROL_EVENT_OFFSET 12
#define SANDBOX_CONTROL_SEQUENCE_OFFSET 16
#define SANDBOX_CONTROL_NUMBER_OFFSET 24
#define SANDBOX_CONTROL_ARGS_OFFSET 32
#define SANDBOX_CONTROL_RESULT_OFFSET 80
#define SANDBOX_CONTROL_ERROR_OFFSET 88
#define SANDBOX_CONTROL_ACTION_OFFSET 92
#define SANDBOX_CONTROL_SIZE 96

#define SANDBOX_LAUNCH_ENTRY_GPA 0xe040
#define SANDBOX_LAUNCH_STACK_GPA 0xe048

#ifndef __ASSEMBLER__
#define SANDBOX_MAGIC 0x53414e44424f5821ULL
struct sandbox_control {
	unsigned long long magic;
	unsigned int version;
	unsigned int event;
	unsigned long long sequence;
	unsigned long long number;
	unsigned long long args[6];
	long long result;
	unsigned int error;
	unsigned int action;
};
#endif

/* Guest-to-host report bytes.  The host may extend this set without changing
 * the register or memory layout used to start a guest. */
#define SANDBOX_REPORT_PRIVILEGE 'P'
#define SANDBOX_REPORT_FAULT 'Q'
#define SANDBOX_REPORT_FAILURE 'F'
#define SANDBOX_REPORT_SYSCALL 'S'
#define SANDBOX_REPORT_EXIT 'X'

#endif
