/* Define the entry point for the payload. */
ENTRY(_start)

SECTIONS {
    /* Set the base load address to 1MB. */
    . = 0x100000;

    /* Executable code. */
    .text : {
        *(.text .text.*)
    }

    /* Read-only data (constants). */
    .rodata : {
        *(.rodata .rodata.*)
    }

    /* Initialized read-write data. */
    .data : {
        *(.data .data.*)
    }

    /* Uninitialized read-write data (zero-filled). */
    .bss : {
        *(.bss .bss.*)
    }
}