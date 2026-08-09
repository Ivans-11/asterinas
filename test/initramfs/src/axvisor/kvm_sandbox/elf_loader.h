/* SPDX-License-Identifier: MPL-2.0 */
#ifndef AXVISOR_KVM_SANDBOX_ELF_LOADER_H
#define AXVISOR_KVM_SANDBOX_ELF_LOADER_H

struct sandbox_image {
	unsigned long long entry;
	unsigned long long image_start;
	unsigned long long image_end;
	unsigned long long program_header_address;
	unsigned short program_header_entry_size;
	unsigned short program_header_count;
	unsigned char page_flags[512];
};

#define SANDBOX_IMAGE_PAGE_READ 1
#define SANDBOX_IMAGE_PAGE_WRITE 2
#define SANDBOX_IMAGE_PAGE_EXEC 4

int sandbox_load_static_elf(const char *path, unsigned char *memory,
			   unsigned long memory_size, unsigned long long load_start,
			   unsigned long long load_end, unsigned short machine,
			   struct sandbox_image *image);

#endif
