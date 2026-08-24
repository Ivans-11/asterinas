// SPDX-License-Identifier: MPL-2.0

#include "elf_loader.h"
#include "runner.h"
#include "syscall_proxy.h"

static int strings_equal(const char *left, const char *right)
{
	while (*left != '\0' && *right != '\0') {
		if (*left++ != *right++)
			return 0;
	}
	return *left == '\0' && *right == '\0';
}

static int sandbox_run_application(struct sandbox_runner *runner,
				   const struct sandbox_backend_ops *backend,
				   volatile struct sandbox_control *control)
{
	struct sandbox_event event;

	for (;;) {
		if (sandbox_runner_next_event(runner, backend, control, &event) != 0)
			return sandbox_fail("sandbox event loop failed\n");
		if (sandbox_service_linux_event(runner, control, &event, backend) != 0)
			return sandbox_fail("sandbox syscall service failed\n");
		if (event.kind == SANDBOX_EVENT_FAULT)
			return sandbox_fail("sandbox application faulted\n");
		if (event.kind == SANDBOX_EVENT_EXIT) {
			if (event.args[0] != 0)
				return sandbox_fail("sandbox application returned a failure status\n");
			break;
		}
	}

	sandbox_print("kvm sandbox pass\n");
	return 0;
}

static int sandbox_watchdog_finish(struct sandbox_runner *runner, int result)
{
	sandbox_unmap(runner->run, runner->run_mmap_size);
	return result;
}

int sandbox_main(long host_argument_count, char *host_arguments[])
{
	static char default_application[] = "/test/kvm_sandbox_app";
	char *default_arguments[] = { default_application };
	const unsigned long watchdog_interval_milliseconds = 10;
	const unsigned long watchdog_poll_limit =
		SANDBOX_DEFAULT_TIMEOUT_SECONDS * 1000 / watchdog_interval_milliseconds;
	const struct sandbox_backend_ops *backend = &sandbox_backend;
	volatile struct sandbox_control *control;
	struct sandbox_image image;
	struct sandbox_runner runner;
	unsigned long long stack_pointer;
	unsigned long poll_count;
	long child_pid;
	long result;
	int status;
	long application_argument_count;
	char **application_arguments;
	const char *application_path;
	const char *allowed_read_files[SANDBOX_MAX_ALLOWED_FILES];
	unsigned int allowed_read_file_count = 0;
	long argument_index = 1;

	while (argument_index < host_argument_count &&
	       strings_equal(host_arguments[argument_index], "--allow-read")) {
		if (allowed_read_file_count >= SANDBOX_MAX_ALLOWED_FILES ||
		    argument_index + 1 >= host_argument_count)
			return sandbox_fail("sandbox file policy is invalid\n");
		allowed_read_files[allowed_read_file_count++] =
			host_arguments[argument_index + 1];
		argument_index += 2;
	}
	if (argument_index < host_argument_count &&
	    strings_equal(host_arguments[argument_index], "--"))
		argument_index++;
	if (argument_index < host_argument_count) {
		application_argument_count = host_argument_count - argument_index;
		application_arguments = &host_arguments[argument_index];
	} else if (host_argument_count == 1) {
		application_argument_count = 1;
		application_arguments = default_arguments;
	} else {
		return sandbox_fail("sandbox application path is missing\n");
	}
	application_path = application_arguments[0];

	sandbox_print("kvm sandbox: start\n");
	if (sandbox_runner_create(&runner, backend->guest_memory_size) != 0)
		return 1;
	runner.allowed_read_file_count = allowed_read_file_count;
	runner.application_path = application_path;
	for (unsigned int index = 0; index < allowed_read_file_count; index++)
		runner.allowed_read_files[index] = allowed_read_files[index];
	if (sandbox_load_static_elf(application_path, runner.guest_memory,
				    runner.guest_memory_size, backend->application_start,
				    backend->stack_bottom, backend->elf_machine, &image) != 0)
		return sandbox_runner_abort(&runner, "sandbox ELF load failed\n");

	runner.guest_brk = image.image_end;
	runner.guest_heap_limit = backend->stack_bottom;
	runner.guest_image_start = image.image_start;
	runner.guest_image_end = image.image_end;
	runner.image = &image;
	runner.guest_stack_bottom = backend->stack_bottom;
	runner.guest_stack_top = backend->stack_top;
	if (sandbox_prepare_initial_stack(&runner, application_argument_count,
					  application_arguments, &image,
					  runner.guest_stack_bottom, runner.guest_stack_top,
					  &stack_pointer) != 0)
		return sandbox_runner_abort(&runner, "sandbox initial stack setup failed\n");
	if (backend->prepare(&runner, &image, stack_pointer) != 0)
		return sandbox_runner_abort(&runner, "sandbox backend setup failed\n");
	control = (volatile struct sandbox_control *)&runner.guest_memory[backend->control_gpa];
	sandbox_print("kvm sandbox: guest ready\n");

	child_pid = sandbox_fork();
	if (child_pid < 0)
		return sandbox_runner_abort(
			&runner, "sandbox supervisor could not create child process\n");
	if (child_pid == 0) {
		/* The watchdog only needs the shared run page.  Release every other
		 * inherited KVM resource before the parent starts running the guest so
		 * wait4 does not race deferred process-exit cleanup. */
		sandbox_close(runner.vcpu_fd);
		sandbox_close(runner.vm_fd);
		sandbox_close(runner.kvm_fd);
		sandbox_unmap(runner.guest_memory, runner.guest_memory_size);
		for (poll_count = 0; poll_count < watchdog_poll_limit; poll_count++) {
			if (sandbox_runner_exit_requested(&runner))
				return sandbox_watchdog_finish(&runner, 0);
			if (sandbox_sleep_milliseconds(watchdog_interval_milliseconds) != 0)
				return sandbox_watchdog_finish(&runner, 1);
		}
		sandbox_runner_request_exit(&runner);
		return sandbox_watchdog_finish(&runner, 0);
	}

	result = sandbox_run_application(&runner, backend, control);
	sandbox_runner_request_exit(&runner);
	if (sandbox_wait(child_pid, &status, 0) != child_pid)
		return sandbox_fail("sandbox supervisor could not reap watchdog\n");
	sandbox_runner_destroy(&runner);
	return result;
}
