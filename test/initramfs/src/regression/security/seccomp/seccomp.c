// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stddef.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef SYS_SECCOMP
#define SYS_SECCOMP 1
#endif

static _Thread_local volatile sig_atomic_t sigsys_count;
static atomic_int worker_ready;
static atomic_int worker_go;

static void fail(const char *message)
{
	perror(message);
	exit(EXIT_FAILURE);
}

static void handle_sigsys(int sig, siginfo_t *info, void *context)
{
	(void)context;
	if (sig == SIGSYS && info->si_code == SYS_SECCOMP)
		sigsys_count++;
}

static void *worker(void *unused)
{
	(void)unused;
	atomic_store_explicit(&worker_ready, 1, memory_order_release);
	while (!atomic_load_explicit(&worker_go, memory_order_acquire))
		sched_yield();

	syscall(SYS_getppid);
	return (void *)(uintptr_t)(sigsys_count == 1);
}

int main(void)
{
	struct sigaction action = {
		.sa_sigaction = handle_sigsys,
		.sa_flags = SA_SIGINFO,
	};
	struct sock_filter instructions[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_TRAP),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog program = {
		.len = sizeof(instructions) / sizeof(instructions[0]),
		.filter = instructions,
	};
	pthread_t thread;
	void *thread_result;
	pid_t child;
	int status;
	unsigned int action_query = SECCOMP_RET_KILL_PROCESS;

	if (prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 0)
		fail("initial PR_GET_NO_NEW_PRIVS");
	if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0)
		fail("PR_SET_NO_NEW_PRIVS");
	if (prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 1)
		fail("final PR_GET_NO_NEW_PRIVS");

	if (syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0,
		    &action_query) != 0)
		fail("SECCOMP_GET_ACTION_AVAIL");
	if (sigaction(SIGSYS, &action, NULL) != 0)
		fail("sigaction SIGSYS");
	if (pthread_create(&thread, NULL, worker, NULL) != 0)
		fail("pthread_create");
	while (!atomic_load_explicit(&worker_ready, memory_order_acquire))
		sched_yield();

	if (syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
		    SECCOMP_FILTER_FLAG_TSYNC, &program) != 0)
		fail("SECCOMP_SET_MODE_FILTER");

	atomic_store_explicit(&worker_go, 1, memory_order_release);
	if (pthread_join(thread, &thread_result) != 0)
		fail("pthread_join");
	if ((uintptr_t)thread_result != 1) {
		fprintf(stderr, "TSYNC filter did not trap the sibling thread\n");
		return EXIT_FAILURE;
	}

	syscall(SYS_getppid);
	if (sigsys_count != 1) {
		fprintf(stderr, "seccomp trap did not deliver SIGSYS\n");
		return EXIT_FAILURE;
	}

	child = fork();
	if (child < 0)
		fail("fork");
	if (child == 0) {
		sigsys_count = 0;
		syscall(SYS_getppid);
		_exit(sigsys_count == 1 ? EXIT_SUCCESS : EXIT_FAILURE);
	}
	if (waitpid(child, &status, 0) != child)
		fail("waitpid");
	if (!WIFEXITED(status) || WEXITSTATUS(status) != EXIT_SUCCESS) {
		fprintf(stderr, "forked child did not inherit the seccomp filter\n");
		return EXIT_FAILURE;
	}

	puts("seccomp test passed");
	return EXIT_SUCCESS;
}
