// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <sched.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define MOUNT_ROOT "/tmp/mount_propagation_root"

FN_TEST(slave_mount_compatibility)
{
	TEST_SUCC(unshare(CLONE_NEWNS));
	CHECK_WITH(mkdir(MOUNT_ROOT, 0755), _ret == 0 || errno == EEXIST);
	TEST_SUCC(mount("tmpfs", MOUNT_ROOT, "tmpfs", 0, NULL));

	TEST_SUCC(mount(NULL, MOUNT_ROOT, NULL, MS_SLAVE | MS_REC, NULL));
	TEST_ERRNO(mount(NULL, MOUNT_ROOT, NULL, MS_SHARED, NULL), EINVAL);

	TEST_SUCC(umount(MOUNT_ROOT));
	TEST_SUCC(rmdir(MOUNT_ROOT));
}
END_TEST()
