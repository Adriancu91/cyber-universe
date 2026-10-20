/* Cyber Universe OS kernel linker script (x86_64, higher half, loaded by Limine). */
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(kmain)

PHDRS
{
    limine_requests PT_LOAD;
    text            PT_LOAD;
    rodata          PT_LOAD;
    data            PT_LOAD;
}

SECTIONS
{
    /* Limine requires the kernel to live in the top 2 GiB of the address space. */
    . = 0xffffffff80000000;
    __kernel_start = .;

    .limine_requests : {
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    } :limine_requests

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .text : { *(.text .text.*) } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .rodata : { *(.rodata .rodata.*) } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : { *(.data .data.*) } :data
    .bss  : { *(.bss .bss.*) *(COMMON) } :data
    . = ALIGN(CONSTANT(MAXPAGESIZE));
    __kernel_end = .;

    /DISCARD/ : { *(.eh_frame*) *(.note .note.*) }
}
