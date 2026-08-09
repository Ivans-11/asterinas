/* SPDX-License-Identifier: MPL-2.0 */
ENTRY(_start)

PHDRS
{
  text PT_LOAD FILEHDR PHDRS FLAGS(5);
  data PT_LOAD FLAGS(6);
}

SECTIONS
{
  . = 0x10000 + SIZEOF_HEADERS;
  .text : { *(.text .text.*) } :text
  .rodata : { *(.rodata .rodata.*) } :text
  . = ALIGN(0x1000);
  __global_pointer$ = . + 0x800;
  .data : { *(.data .data.*) } :data
  .bss : { *(.bss .bss.* COMMON) } :data
  /DISCARD/ : { *(.comment .note.* .riscv.attributes) }
}
