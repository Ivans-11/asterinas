// SPDX-License-Identifier: MPL-2.0

#include "elf_loader.h"
#include "runner.h"

#define ELFCLASS64 2
#define ELFDATA2LSB 1
#define ET_EXEC 2
#define PT_LOAD 1
#define PT_INTERP 3
#define EI_CLASS 4
#define EI_DATA 5
#define EI_NIDENT 16
#define PF_X 1
#define PF_W 2
#define PF_R 4
#define ELF_HEADER_SIZE 64
#define ELF_PHDR_SIZE 56
#define MAX_PHNUM 32
#define PAGE_SIZE 0x1000UL

struct elf64_header {
	unsigned char ident[EI_NIDENT];
	unsigned short type;
	unsigned short machine;
	unsigned int version;
	unsigned long long entry;
	unsigned long long phoff;
	unsigned long long shoff;
	unsigned int flags;
	unsigned short ehsize;
	unsigned short phentsize;
	unsigned short phnum;
	unsigned short shentsize;
	unsigned short shnum;
	unsigned short shstrndx;
};

struct elf64_phdr {
	unsigned int type;
	unsigned int flags;
	unsigned long long offset;
	unsigned long long vaddr;
	unsigned long long paddr;
	unsigned long long filesz;
	unsigned long long memsz;
	unsigned long long align;
};

static int read_exact(long fd, void *buffer, unsigned long length)
{
	unsigned char *cursor = buffer;

	while (length) {
		long count = sandbox_read(fd, cursor, length);
		if (count <= 0)
			return 1;
		cursor += count;
		length -= (unsigned long)count;
	}
	return 0;
}

int sandbox_load_static_elf(const char *path, unsigned char *memory,
			   unsigned long memory_size, unsigned long long load_start,
			   unsigned long long load_end, unsigned short machine,
			   struct sandbox_image *image)
{
	struct elf64_header header;
	struct elf64_phdr phdrs[MAX_PHNUM];
	unsigned long page_count = memory_size / PAGE_SIZE;
	unsigned long long image_start = ~0ULL;
	unsigned long long image_end = 0;
	int entry_is_executable = 0;
	long fd = sandbox_open(path);

	sandbox_zero(image, sizeof(*image));
	if (page_count > sizeof(image->page_flags))
		return sandbox_fail("sandbox guest memory is too large\n");

	if (fd < 0)
		return sandbox_fail("sandbox ELF open failed\n");
	if (read_exact(fd, &header, sizeof(header)) != 0 ||
	    header.ident[0] != 0x7f || header.ident[1] != 'E' || header.ident[2] != 'L' ||
	    header.ident[3] != 'F' || header.ident[EI_CLASS] != ELFCLASS64 ||
	    header.ident[EI_DATA] != ELFDATA2LSB || header.type != ET_EXEC ||
	    header.machine != machine || header.ehsize != ELF_HEADER_SIZE ||
	    header.phentsize != ELF_PHDR_SIZE || header.phnum == 0 || header.phnum > MAX_PHNUM ||
	    header.phoff > 0x100000000ULL) {
		sandbox_close(fd);
		return sandbox_fail("sandbox ELF header is unsupported\n");
	}
	if (sandbox_seek(fd, header.phoff) < 0 ||
	    read_exact(fd, phdrs, (unsigned long)header.phnum * sizeof(phdrs[0])) != 0) {
		sandbox_close(fd);
		return sandbox_fail("sandbox ELF program headers are unreadable\n");
	}

	for (unsigned int index = 0; index < header.phnum; index++) {
		struct elf64_phdr *phdr = &phdrs[index];
		unsigned long long end;

		if (phdr->type == PT_INTERP) {
			sandbox_close(fd);
			return sandbox_fail("sandbox ELF must be statically linked\n");
		}
		if (phdr->type != PT_LOAD)
			continue;
		if (phdr->filesz > phdr->memsz || phdr->vaddr < load_start ||
		    phdr->vaddr > load_end || phdr->memsz > load_end - phdr->vaddr ||
		    phdr->vaddr > memory_size ||
		    phdr->memsz > memory_size - phdr->vaddr ||
		    phdr->offset > 0x100000000ULL ||
		    phdr->filesz > 0x100000000ULL - phdr->offset ||
		    (phdr->flags & (PF_W | PF_X)) == (PF_W | PF_X) ||
		    (phdr->vaddr & (PAGE_SIZE - 1)) != (phdr->offset & (PAGE_SIZE - 1))) {
			sandbox_close(fd);
			return sandbox_fail("sandbox ELF segment is invalid\n");
		}
		end = phdr->vaddr + phdr->memsz;
		if (phdr->memsz == 0)
			continue;
		for (unsigned int previous = 0; previous < index; previous++) {
			struct elf64_phdr *other = &phdrs[previous];
			unsigned long long other_end;

			if (other->type != PT_LOAD || other->memsz == 0)
				continue;
			other_end = other->vaddr + other->memsz;
			if (phdr->vaddr < other_end && other->vaddr < end) {
				sandbox_close(fd);
				return sandbox_fail("sandbox ELF segments overlap\n");
			}
		}
		if ((phdr->flags & PF_X) && header.entry >= phdr->vaddr && header.entry < end)
			entry_is_executable = 1;
		if (header.phoff >= phdr->offset &&
		    header.phoff - phdr->offset <= phdr->filesz &&
		    (unsigned long long)header.phnum * header.phentsize <=
			phdr->filesz - (header.phoff - phdr->offset))
			image->program_header_address =
				phdr->vaddr + (header.phoff - phdr->offset);
		for (unsigned long long address = phdr->vaddr & ~(PAGE_SIZE - 1);
		     address < end; address += PAGE_SIZE) {
			unsigned long page = (unsigned long)(address / PAGE_SIZE);
			if (phdr->flags & PF_R)
				image->page_flags[page] |= SANDBOX_IMAGE_PAGE_READ;
			if (phdr->flags & PF_W)
				image->page_flags[page] |= SANDBOX_IMAGE_PAGE_WRITE;
			if (phdr->flags & PF_X)
				image->page_flags[page] |= SANDBOX_IMAGE_PAGE_EXEC;
		}
		if (phdr->vaddr < image_start)
			image_start = phdr->vaddr;
		if (end > image_end)
			image_end = end;
		sandbox_zero(&memory[phdr->vaddr], (unsigned long)phdr->memsz);
		if (sandbox_seek(fd, phdr->offset) < 0 ||
		    read_exact(fd, &memory[phdr->vaddr], (unsigned long)phdr->filesz) != 0) {
			sandbox_close(fd);
			return sandbox_fail("sandbox ELF segment is unreadable\n");
		}
	}
	sandbox_close(fd);
	if (image_start == ~0ULL || !entry_is_executable || header.entry < image_start ||
	    header.entry >= image_end || header.entry >= memory_size)
		return sandbox_fail("sandbox ELF entry point is invalid\n");
	for (unsigned long page = 0; page < page_count; page++)
		if ((image->page_flags[page] &
		     (SANDBOX_IMAGE_PAGE_WRITE | SANDBOX_IMAGE_PAGE_EXEC)) ==
		    (SANDBOX_IMAGE_PAGE_WRITE | SANDBOX_IMAGE_PAGE_EXEC))
			return sandbox_fail("sandbox ELF page is writable and executable\n");
	image->entry = header.entry;
	image->image_start = image_start & ~(PAGE_SIZE - 1);
	image->image_end = (image_end + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1);
	image->program_header_entry_size = header.phentsize;
	image->program_header_count = header.phnum;
	return 0;
}
