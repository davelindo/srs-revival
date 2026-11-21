    .syntax unified
    .arch armv5te
    .arm
    .text
    .balign 4

    /* Replaces ldr r3, [r3, #0x24] in sulu_eth_isr. r2 is driver private data. */
    .global x7_eth_read_isr
    .type x7_eth_read_isr, %function
x7_eth_read_isr:
    ldr r3, [r3, #0x24]
    tst r3, #4                     /* Receive buffer unavailable. */
    bxeq lr
    push {r0, r1, r2, r4, r12, lr}
    mov r4, r3
    mov r0, r2
    bl x7_rx_recover
    cmp r0, #0
    bicne r4, r4, #2
    mov r3, r4
    pop {r0, r1, r2, r4, r12, pc}
    .size x7_eth_read_isr, . - x7_eth_read_isr

    /* Same callback ABI as sulu_rxdma_handler, with r1 carrying net_device. */
    .global x7_eth_rx_dma
    .type x7_eth_rx_dma, %function
x7_eth_rx_dma:
    push {r4, r5, r6, lr}
    mov r5, r1
    mrs r4, cpsr
    orr r12, r4, #0x80
    msr cpsr_c, r12
    ldr r12, =0xc014bc9c
    blx r12
    mov r6, r0
    ldr r0, [r5, #0x194]
    bl x7_rx_recover
    mov r0, r6
    msr cpsr_c, r4
    pop {r4, r5, r6, pc}
    .size x7_eth_rx_dma, . - x7_eth_rx_dma
