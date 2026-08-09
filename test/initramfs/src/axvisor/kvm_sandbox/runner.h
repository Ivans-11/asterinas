/* SPDX-License-Identifier: MPL-2.0 */
#ifndef AXVISOR_KVM_SANDBOX_RUNNER_H
#define AXVISOR_KVM_SANDBOX_RUNNER_H

#include "sandbox_abi.h"

#define SANDBOX_KVM_API_VERSION 12
#define SANDBOX_DEFAULT_EVENT_LIMIT 4096
#define SANDBOX_DEFAULT_IO_LIMIT (1024UL * 1024UL)
#define SANDBOX_DEFAULT_TIMEOUT_SECONDS 2
#define SANDBOX_USER_STACK_SIZE (256UL * 1024UL)
#define SANDBOX_MAX_ARGUMENTS 16
#define SANDBOX_MAX_GUEST_FDS 16
#define SANDBOX_MAX_ALLOWED_FILES 4

struct kvm_userspace_memory_region {
	unsigned int slot;
	unsigned int flags;
	unsigned long long guest_phys_addr;
	unsigned long long memory_size;
	unsigned long long userspace_addr;
};

struct sandbox_image;

struct sandbox_runner {
	long kvm_fd;
	long vm_fd;
	long vcpu_fd;
	long run_mmap_size;
	unsigned long guest_memory_size;
	unsigned int exit_count;
	unsigned int exit_limit;
	unsigned int memory_registered;
	unsigned long io_bytes;
	unsigned long io_limit;
	unsigned long long guest_brk;
	unsigned long long guest_heap_limit;
	unsigned long long guest_image_start;
	unsigned long long guest_image_end;
	unsigned long long guest_stack_bottom;
	unsigned long long guest_stack_top;
	long guest_host_fds[SANDBOX_MAX_GUEST_FDS];
	const char *allowed_read_files[SANDBOX_MAX_ALLOWED_FILES];
	unsigned int allowed_read_file_count;
	const char *application_path;
	const struct sandbox_image *image;
	unsigned char *guest_memory;
	void *run;
};

struct sandbox_event {
	unsigned int kind;
	unsigned long long number;
	unsigned long long args[6];
};

struct sandbox_linux_abi;

struct sandbox_backend_ops {
	unsigned long guest_memory_size;
	unsigned long application_start;
	unsigned long stack_bottom;
	unsigned long stack_top;
	unsigned long control_gpa;
	unsigned short elf_machine;
	const struct sandbox_linux_abi *linux_abi;
	int (*prepare)(struct sandbox_runner *runner,
		       const struct sandbox_image *image,
		       unsigned long long stack_pointer);
	int (*decode_exit)(struct sandbox_runner *runner, unsigned int *event_kind);
	int (*service_event)(struct sandbox_runner *runner,
			     volatile struct sandbox_control *control,
			     const struct sandbox_event *event);
};

extern const struct sandbox_backend_ops sandbox_backend;

long sandbox_ioctl(long fd, unsigned long request, unsigned long argument);
void sandbox_print(const char *message);
int sandbox_fail(const char *message);
long sandbox_open(const char *path);
long sandbox_read(long fd, void *buffer, unsigned long length);
long sandbox_write(long fd, const void *buffer, unsigned long length);
long sandbox_seek(long fd, unsigned long long offset);
long sandbox_fstat(long fd, void *stat_buffer);
long sandbox_clock_gettime(long clock_id, void *timespec);
long sandbox_fork(void);
long sandbox_wait(long pid, int *status, int nohang);
int sandbox_sleep_milliseconds(unsigned long milliseconds);
long sandbox_close(long fd);
void sandbox_copy(void *destination, const void *source, unsigned long length);
void sandbox_zero(void *destination, unsigned long length);
int sandbox_prepare_initial_stack(struct sandbox_runner *runner, long argument_count,
				  char *const arguments[],
				  const struct sandbox_image *image,
				  unsigned long long stack_bottom,
				  unsigned long long stack_top,
				  unsigned long long *stack_pointer);

int sandbox_runner_create(struct sandbox_runner *runner, unsigned long guest_memory_size);
void sandbox_runner_destroy(struct sandbox_runner *runner);
void sandbox_runner_request_exit(struct sandbox_runner *runner);
int sandbox_runner_exit_requested(const struct sandbox_runner *runner);
int sandbox_runner_abort(struct sandbox_runner *runner, const char *message);
int sandbox_runner_next_event(struct sandbox_runner *runner,
			      const struct sandbox_backend_ops *backend,
			      volatile const struct sandbox_control *control,
			      struct sandbox_event *event);

#endif
