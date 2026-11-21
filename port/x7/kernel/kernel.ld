/* Keep Linux's metadata while removing unused Rust/core code. This is a
 * relocatable .ko, not an executable and not an image for the controller. */
SECTIONS
{
  .modinfo : { KEEP(*(.modinfo)) }
  .gnu.linkonce.this_module : { KEEP(*(.gnu.linkonce.this_module)) }
  /DISCARD/ : { *(.ARM.exidx* .ARM.extab* .eh_frame*) }
}
