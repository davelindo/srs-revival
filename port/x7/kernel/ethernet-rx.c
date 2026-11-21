/* MACB receive-buffer exhaustion recovery for the retained DM870 kernel. */
#include <stdint.h>

enum {
    RX_DESCRIPTORS = 96,
    TX_RING_BYTES = 8 * 8,
    MAC_CONTROL = 0x00 / 4,
    MAC_RX_QUEUE = 0x18 / 4,
    MAC_RX_STATUS = 0x20 / 4,
    RX_ENABLE = 1 << 2,
    RX_BUFFER_UNAVAILABLE = 1,
    RX_OWNED = 1,
    RX_WRAP = 2,
    RX_START_OF_FRAME = 1 << 14,
};

struct rx_descriptor {
    uint32_t address;
    uint32_t status;
};

struct rx_state {
    volatile uint32_t *mac;
    volatile struct rx_descriptor *ring;
    uint32_t bus_address;
    volatile uint32_t *scan_index;
    volatile uint32_t *reclaim_index;
    volatile uint32_t *drops;
    uint32_t pending;
    uint32_t active;
};

static inline void drain_writes(void) {
#if defined(__arm__)
    // ARM926 write buffer drain, including accesses through the SRAM alias.
    __asm__ volatile("mcr p15, 0, %0, c7, c10, 4" : : "r"(0) : "memory");
#endif
}

/* Return 1 to skip this IRQ's normal receive scan while the ring is blocked. */
static int recover_receive(const struct rx_state *rx) {
    volatile uint32_t *mac = rx->mac;
    if (!(mac[MAC_RX_STATUS] & RX_BUFFER_UNAVAILABLE))
        return 0;
    uint32_t control = mac[MAC_CONTROL];
    if (!(control & RX_ENABLE) || rx->pending || rx->active)
        return 1;

    uint32_t head = mac[MAC_RX_QUEUE] - rx->bus_address;
    if (head >= RX_DESCRIPTORS * sizeof *rx->ring || (head & 7) || !(rx->ring[RX_DESCRIPTORS - 1].address & RX_WRAP))
        return 1;
    if (!(rx->ring[head / sizeof *rx->ring].address & RX_OWNED)) {
        // The DMA completion has already made space. Keep any queued frames.
        mac[MAC_RX_STATUS] = RX_BUFFER_UNAVAILABLE;
        return 0;
    }

    // The MAC cannot write descriptors with RX disabled, and the driver's RX
    // DMA queue has drained. Leave the transmitter and link state in place.
    mac[MAC_CONTROL] = control & ~RX_ENABLE;
    (void)mac[MAC_CONTROL];
    drain_writes();
    unsigned dropped = 0;
    for (unsigned i = 0; i < RX_DESCRIPTORS; ++i) {
        uint32_t address = rx->ring[i].address;
        if ((address & RX_OWNED) && (rx->ring[i].status & RX_START_OF_FRAME))
            ++dropped;
        rx->ring[i].status = 0;
        rx->ring[i].address = address & ~RX_OWNED;
    }
    *rx->drops += dropped;
    *rx->scan_index = 0;
    *rx->reclaim_index = 0;
    drain_writes();
    mac[MAC_RX_QUEUE] = rx->bus_address;
    mac[MAC_RX_STATUS] = RX_BUFFER_UNAVAILABLE;
    mac[MAC_CONTROL] = control;
    return 1;
}

#ifndef X7_RX_HOST_TEST
// The offsets come from sulu_hw_start, sulu_eth_isr and sulu_rxdma_handler.
enum {
    PRIV_MAC = 0x00 / 4,
    PRIV_TX_RING_BUS = 0x08 / 4,
    PRIV_RX_RING_CPU = 0x0c / 4,
    PRIV_RX_SCAN = 0x1c / 4,
    PRIV_RX_RECLAIM = 0x20 / 4,
    PRIV_RX_DROPPED = 0x1e4 / 4,
    PRIV_DMA_ACTIVE = 0x146c / 4,
    PRIV_DMA_PENDING = 0x1470 / 4,
};

// Both hooks enter with local IRQs disabled so the queue cannot change here.
int x7_rx_recover(volatile uint32_t *priv) {
    volatile uint32_t *mac = (volatile uint32_t *)(uintptr_t)priv[PRIV_MAC];
    if (!(mac[MAC_RX_STATUS] & RX_BUFFER_UNAVAILABLE))
        return 0;
    const struct rx_state rx = {
        .mac = mac,
        .ring = (volatile struct rx_descriptor *)(uintptr_t)priv[PRIV_RX_RING_CPU],
        .bus_address = priv[PRIV_TX_RING_BUS] + TX_RING_BYTES,
        .scan_index = priv + PRIV_RX_SCAN,
        .reclaim_index = priv + PRIV_RX_RECLAIM,
        .drops = priv + PRIV_RX_DROPPED,
        .pending = priv[PRIV_DMA_PENDING],
        .active = priv[PRIV_DMA_ACTIVE],
    };
    return recover_receive(&rx);
}
#endif
