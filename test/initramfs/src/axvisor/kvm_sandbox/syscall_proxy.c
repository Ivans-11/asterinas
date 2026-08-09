// SPDX-License-Identifier: MPL-2.0

#include "syscall_proxy.h"
#include "elf_loader.h"

static int translate_guest_range(struct sandbox_runner *runner,
				 unsigned long long address,
				 unsigned long long length,
				 int write_to_guest,
				 unsigned char **host_address)
{
	if (address >= runner->guest_memory_size ||
	    length > runner->guest_memory_size - address)
		return 1;
	if (address >= runner->guest_image_start && address <= runner->guest_image_end &&
	    length <= runner->guest_image_end - address) {
		if (!write_to_guest || length == 0)
			goto valid;
		for (unsigned long long page = address >> 12;
		     page <= (address + length - 1) >> 12; page++)
			if (page >= sizeof(runner->image->page_flags) ||
			    !(runner->image->page_flags[page] & SANDBOX_IMAGE_PAGE_WRITE))
				return 1;
		goto valid;
	}
	if (address >= runner->guest_image_end && address <= runner->guest_brk &&
	    length <= runner->guest_brk - address)
		goto valid;
	if (address >= runner->guest_stack_bottom && address <= runner->guest_stack_top &&
	    length <= runner->guest_stack_top - address)
		goto valid;
	return 1;
valid:
	*host_address = &runner->guest_memory[address];
	return 0;
}

static int account_io(struct sandbox_runner *runner, unsigned long long length)
{
	if (length > runner->io_limit - runner->io_bytes)
		return sandbox_fail("sandbox application exceeded I/O budget\n");
	runner->io_bytes += (unsigned long)length;
	return 0;
}

static int guest_cstring(struct sandbox_runner *runner, unsigned long long address,
			 unsigned char **host_address)
{
	unsigned char *character;
	unsigned long length;

	for (length = 0; length < 256; length++) {
		if (address >= runner->guest_memory_size ||
		    length >= runner->guest_memory_size - address)
			return 1;
		if (translate_guest_range(runner, address + length, 1, 0, &character) != 0)
			return 1;
		if (*character == '\0') {
			*host_address = &runner->guest_memory[address];
			return 0;
		}
	}
	return 1;
}

static int strings_equal(const unsigned char *left, const char *right)
{
	while (*left != '\0' && *right != '\0') {
		if (*left++ != (unsigned char)*right++)
			return 0;
	}
	return *left == '\0' && *right == '\0';
}

static unsigned long string_length(const char *string)
{
	unsigned long length = 0;

	while (string[length] != '\0')
		length++;
	return length;
}

static int path_is_allowed(struct sandbox_runner *runner, const unsigned char *path)
{
	unsigned int index;

	for (index = 0; index < runner->allowed_read_file_count; index++) {
		if (strings_equal(path, runner->allowed_read_files[index]))
			return 1;
	}
	return 0;
}

static long guest_host_fd(struct sandbox_runner *runner, unsigned long long guest_fd)
{
	if (guest_fd >= SANDBOX_MAX_GUEST_FDS)
		return -1;
	return runner->guest_host_fds[guest_fd];
}

static long allocate_guest_fd(struct sandbox_runner *runner, long host_fd)
{
	unsigned int guest_fd;

	for (guest_fd = 3; guest_fd < SANDBOX_MAX_GUEST_FDS; guest_fd++) {
		if (runner->guest_host_fds[guest_fd] < 0) {
			runner->guest_host_fds[guest_fd] = host_fd;
			return guest_fd;
		}
	}
	return -1;
}

static void complete_syscall(volatile struct sandbox_control *control, long result)
{
	control->result = result;
	control->error = result < 0 ? (int)-result : 0;
	control->action = SANDBOX_ACTION_RESUME;
}

int sandbox_service_linux_event(struct sandbox_runner *runner,
				volatile struct sandbox_control *control,
				const struct sandbox_event *event,
				const struct sandbox_backend_ops *backend)
{
	const struct sandbox_linux_abi *abi = backend->linux_abi;

	if (event->kind == SANDBOX_EVENT_FAULT) {
		if (event->args[0] == 2)
			sandbox_fail("sandbox guest raised an illegal instruction\n");
		else if (event->args[0] == 12)
			sandbox_fail("sandbox guest raised an instruction page fault\n");
		else if (event->args[0] == 13)
			sandbox_fail("sandbox guest raised a load page fault\n");
		else if (event->args[0] == 15)
			sandbox_fail("sandbox guest raised a store page fault\n");
		control->result = -14;
		control->error = 14;
		control->action = SANDBOX_ACTION_TERMINATE;
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->write_number) {
		unsigned char *buffer;

		if (event->args[0] != 1 ||
		    translate_guest_range(runner, event->args[1], event->args[2], 0, &buffer) != 0)
			return sandbox_fail("sandbox write arguments are invalid\n");
		if (account_io(runner, event->args[2]) != 0)
			return 1;
		control->result = sandbox_write(1, buffer, event->args[2]);
		control->error = 0;
		control->action = SANDBOX_ACTION_RESUME;
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->read_number) {
		unsigned char *buffer;
		long host_fd = guest_host_fd(runner, event->args[0]);

		if (host_fd < 0 ||
		    translate_guest_range(runner, event->args[1], event->args[2], 1, &buffer) != 0)
			return sandbox_fail("sandbox read arguments are invalid\n");
		if (account_io(runner, event->args[2]) != 0)
			return 1;
		complete_syscall(control, sandbox_read(host_fd, buffer, event->args[2]));
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->openat_number) {
		unsigned char *path;
		long guest_fd;
		long host_fd;

		if ((int)(unsigned int)event->args[0] != -100 || event->args[2] != 0 ||
		    guest_cstring(runner, event->args[1], &path) != 0)
			return sandbox_fail("sandbox openat arguments are invalid\n");
		if (!path_is_allowed(runner, path)) {
			complete_syscall(control, -13);
			return 0;
		}
		host_fd = sandbox_open((const char *)path);
		if (host_fd < 0) {
			complete_syscall(control, host_fd);
			return 0;
		}
		guest_fd = allocate_guest_fd(runner, host_fd);
		if (guest_fd < 0) {
			sandbox_close(host_fd);
			complete_syscall(control, -24);
			return 0;
		}
		complete_syscall(control, guest_fd);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->close_number) {
		unsigned long long guest_fd = event->args[0];
		long host_fd = guest_host_fd(runner, guest_fd);

		if (guest_fd < 3 || host_fd < 0) {
			complete_syscall(control, -9);
			return 0;
		}
		runner->guest_host_fds[guest_fd] = -1;
		complete_syscall(control, sandbox_close(host_fd));
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->lseek_number) {
		long host_fd = guest_host_fd(runner, event->args[0]);

		if (host_fd < 0 || event->args[2] != 0) {
			complete_syscall(control, -22);
			return 0;
		}
		complete_syscall(control, sandbox_seek(host_fd, event->args[1]));
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->fstat_number) {
		unsigned char *guest_stat;
		unsigned char host_stat[144];
		long host_fd = guest_host_fd(runner, event->args[0]);

		if (host_fd < 0 || abi->fstat_size > sizeof(host_stat) ||
		    translate_guest_range(runner, event->args[1], abi->fstat_size, 1,
					  &guest_stat) != 0)
			return sandbox_fail("sandbox fstat arguments are invalid\n");
		sandbox_zero(host_stat, sizeof(host_stat));
		complete_syscall(control, sandbox_fstat(host_fd, host_stat));
		if (control->result == 0)
			sandbox_copy(guest_stat, host_stat, abi->fstat_size);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL &&
	    event->number == abi->clock_gettime_number) {
		unsigned char *timespec;

		if (translate_guest_range(runner, event->args[1], 16, 1, &timespec) != 0)
			return sandbox_fail("sandbox clock_gettime arguments are invalid\n");
		control->result = sandbox_clock_gettime(event->args[0], timespec);
		control->error = 0;
		control->action = SANDBOX_ACTION_RESUME;
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->brk_number) {
		unsigned long long requested = event->args[0];

		if (requested != 0 && requested >= runner->guest_image_end &&
		    requested <= runner->guest_heap_limit)
			runner->guest_brk = requested;
		control->result = runner->guest_brk;
		control->error = 0;
		control->action = SANDBOX_ACTION_RESUME;
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL &&
	    event->number == abi->set_tid_address_number) {
		complete_syscall(control, (long)event->args[0]);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL &&
	    event->number == abi->set_robust_list_number) {
		complete_syscall(control, 0);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->rseq_number) {
		/* A single-threaded sandbox has no restartable-sequence registration. */
		complete_syscall(control, -38);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL &&
	    event->number == abi->rt_sigprocmask_number) {
		unsigned char *old_mask;

		if (event->args[3] != 8)
			return sandbox_fail("sandbox rt_sigprocmask arguments are invalid\n");
		if (event->args[2] != 0) {
			if (translate_guest_range(runner, event->args[2], 8, 1, &old_mask) != 0)
				return sandbox_fail(
					"sandbox rt_sigprocmask arguments are invalid\n");
			sandbox_zero(old_mask, 8);
		}
		complete_syscall(control, 0);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->prlimit64_number) {
		unsigned char *limits;

		if (event->args[0] != 0 || event->args[1] != 3 || event->args[2] != 0 ||
		    event->args[3] == 0 ||
		    translate_guest_range(runner, event->args[3], 16, 1, &limits) != 0)
			return sandbox_fail("sandbox prlimit64 arguments are invalid\n");
		*(unsigned long long *)&limits[0] =
			runner->guest_stack_top - runner->guest_stack_bottom;
		*(unsigned long long *)&limits[8] =
			runner->guest_stack_top - runner->guest_stack_bottom;
		complete_syscall(control, 0);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->readlinkat_number) {
		unsigned char *path;
		unsigned char *buffer;
		unsigned long length;

		if ((int)(unsigned int)event->args[0] != -100)
			return sandbox_fail("sandbox readlinkat dirfd is invalid\n");
		if (guest_cstring(runner, event->args[1], &path) != 0)
			return sandbox_fail("sandbox readlinkat path pointer is invalid\n");
		if (!strings_equal(path, "/proc/self/exe"))
			return sandbox_fail("sandbox readlinkat path is invalid\n");
		if (translate_guest_range(runner, event->args[2], event->args[3], 1,
				  &buffer) != 0)
			return sandbox_fail("sandbox readlinkat buffer is invalid\n");
		length = string_length(runner->application_path);
		if (length > event->args[3])
			length = (unsigned long)event->args[3];
		sandbox_copy(buffer, runner->application_path, length);
		complete_syscall(control, (long)length);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->mprotect_number) {
		if (event->args[2] != 1 || (event->args[0] & 0xfff) != 0 ||
		    (event->args[1] & 0xfff) != 0 || event->args[0] < runner->guest_image_start ||
		    event->args[0] > runner->guest_image_end ||
		    event->args[1] > runner->guest_image_end - event->args[0])
			return sandbox_fail("sandbox mprotect arguments are invalid\n");
		/* Static libc uses this only to make its already non-executable RELRO pages read-only. */
		complete_syscall(control, 0);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->riscv_hwprobe_number) {
		/* Let libc fall back to the baseline ISA when the sandbox does not expose
		 * a complete hardware-probe implementation. */
		complete_syscall(control, -38);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_SYSCALL && event->number == abi->getrandom_number) {
		struct {
			long seconds;
			long nanoseconds;
		} now;
		unsigned char *buffer;
		unsigned long long state;

		if (event->args[1] > 256 || (event->args[2] & ~1ULL) != 0 ||
		    translate_guest_range(runner, event->args[0], event->args[1], 1,
					  &buffer) != 0)
			return sandbox_fail("sandbox getrandom arguments are invalid\n");
		if (sandbox_clock_gettime(1, &now) != 0)
			return sandbox_fail("sandbox getrandom clock failed\n");
		state = (unsigned long long)now.seconds ^
			((unsigned long long)now.nanoseconds << 32) ^ event->args[0] ^
			runner->exit_count;
		for (unsigned long long index = 0; index < event->args[1]; index++) {
			state ^= state << 13;
			state ^= state >> 7;
			state ^= state << 17;
			buffer[index] = (unsigned char)state;
		}
		complete_syscall(control, (long)event->args[1]);
		return 0;
	}
	if (event->kind == SANDBOX_EVENT_EXIT &&
	    (event->number == abi->exit_number || event->number == abi->exit_group_number)) {
		control->result = (long long)event->args[0];
		control->error = 0;
		control->action = SANDBOX_ACTION_TERMINATE;
		return 0;
	}
	if (backend->service_event != 0) {
		int result = backend->service_event(runner, control, event);

		if (result < 0)
			return 1;
		if (result == 0)
			return 0;
	}
	return sandbox_fail("sandbox syscall is unsupported\n");
}
