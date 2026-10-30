// Behavioral SDRAM model for the Verilator interactive simulator.
//
// This model implements the complete mem_* interface consumed by UNIT-007
// (Memory Arbiter) with timing fidelity matching the W9825G6KH SDRAM:
//   - CAS latency CL=3
//   - Row activation tRCD=2 cycles
//   - Periodic auto-refresh: mem_ready deassertion (~1 per 781 cycles)
//   - Burst cancel/PRECHARGE sequencing (tPRECHARGE=2 cycles)
//
// This model is separate from the zero-latency test harness model at
// spi_gpu/tests/harness/sdram_model.h. An incorrectly timed model will
// mask prefetch FSM and texture cache timing hazards as documented in
// UNIT-007 and UNIT-008.
//
// References:
//   UNIT-007 (Memory Arbiter) -- SDRAM Behavioral Model spec
//   UNIT-008 (Display Controller) -- Scanline prefetch timing
//   UNIT-006 (Pixel Pipeline) -- Texture cache fill timing

#pragma once

#include <cstdint>
#include <unordered_map>

/// SDRAM behavioral model state machine states.
enum class SdramState : uint8_t {
    IDLE,        ///< Waiting for mem_req
    ACTIVATE,    ///< Row activation (tRCD=2 cycles)
    READ_CAS,    ///< CAS latency wait (CL=3 cycles)
    READ_BURST,  ///< Delivering burst read data (1 word/cycle)
    WRITE_BURST, ///< Accepting burst write data (1 word/cycle)
    PRECHARGE,   ///< PRECHARGE delay after burst cancel (2 cycles)
    REFRESH,     ///< Auto-refresh (6 cycles, mem_ready deasserted)
};

/// Behavioral SDRAM model for the Verilator interactive simulator.
///
/// Implements the W9825G6KH SDRAM controller interface with cycle-accurate
/// timing for CAS latency, row activation, auto-refresh, and burst
/// cancel/PRECHARGE sequencing.
///
/// Memory is stored sparsely using an unordered_map to avoid allocating
/// a full 32 MB array when only a fraction of addresses are used.
class SdramModelSim {
public:
    // -- Timing constants (W9825G6KH at 100 MHz) --

    /// Row activation latency in clock cycles (tRCD).
    static constexpr int TRCD = 2;

    /// CAS latency in clock cycles (CL=3).
    static constexpr int CAS_LATENCY = 3;

    /// PRECHARGE delay in clock cycles (tRP).
    static constexpr int TPRECHARGE = 2;

    /// Auto-refresh interval in clock cycles.
    /// 8192 refreshes per 64 ms at 100 MHz = 781.25 cycles per refresh.
    static constexpr int REFRESH_INTERVAL = 781;

    /// Auto-refresh duration in clock cycles.
    static constexpr int REFRESH_DURATION = 6;

    /// Total number of 16-bit words in 32 MB SDRAM.
    static constexpr uint32_t TOTAL_WORDS = 32 * 1024 * 1024 / 2;

    // -- Input signals (set by the testbench / Verilator wrapper before eval) --

    uint8_t mem_req = 0;          ///< Memory access request
    uint8_t mem_we = 0;           ///< Write enable (0=read, 1=write)
    uint32_t mem_addr = 0;        ///< Byte address (24-bit)
    uint32_t mem_wdata = 0;       ///< Write data (single-word, 32-bit)
    uint8_t mem_burst_len = 0;    ///< Burst length in 16-bit words (0=single)
    uint16_t mem_burst_wdata = 0; ///< Write data (burst mode, 16-bit)
    uint8_t mem_burst_cancel = 0; ///< Cancel active burst

    // -- Output signals (driven by the model after eval) --

    uint16_t mem_rdata = 0;           ///< Read data (16-bit, burst mode)
    uint32_t mem_rdata_32 = 0;        ///< Assembled 32-bit read (single-word)
    uint8_t mem_ack = 0;              ///< Access complete
    uint8_t mem_ready = 1;            ///< Ready for new request
    uint8_t mem_burst_data_valid = 0; ///< Valid 16-bit word available (burst read)
    uint8_t mem_burst_wdata_req = 0;  ///< Request next 16-bit write word
    uint8_t mem_burst_done = 0;       ///< Burst transfer complete

    /// Construct the SDRAM behavioral model.
    SdramModelSim();

    /// Evaluate one clock cycle of the SDRAM model.
    ///
    /// Must be called once per rising clock edge. Updates all output
    /// signals based on current input signals and internal state.
    ///
    /// @param sim_time  Current simulation time (reserved for future diagnostics).
    void eval([[maybe_unused]] uint64_t sim_time);

    /// Read a 32-bit value from two consecutive 16-bit words at the given
    /// byte address. Useful for framebuffer readback in the sim app.
    ///
    /// @param byte_addr  Byte address (must be 2-byte aligned for each word).
    /// @return  Assembled 32-bit value (low word at byte_addr, high word at byte_addr+2).
    uint32_t read_word32(uint32_t byte_addr) const;

    /// Read a single 16-bit word at the given word address.
    /// Returns 0 for unwritten addresses (sparse model).
    ///
    /// @param word_addr  16-bit word address.
    /// @return  The stored 16-bit value, or 0 if not yet written.
    uint16_t read_word(uint32_t word_addr) const;

    /// Write a single 16-bit word at the given word address.
    /// Silently ignores out-of-range addresses (>= TOTAL_WORDS).
    ///
    /// @param word_addr  16-bit word address.
    /// @param data       16-bit value to store.
    void write_word(uint32_t word_addr, uint16_t data);

    // Framebuffer readback for the interactive sim is performed by Lua scripts using
    // the block-tiled address formula from INT-011. WIDTH_LOG2 is passed from
    // the fb_config Lua call. No C++ readback helper is required here.

    /// Reset all internal state (state machine, counters, outputs).
    void reset();

    /// Return the current state machine state (for test inspection).
    SdramState current_state() const {
        return state_;
    }

    /// Return the current refresh counter value (for test inspection).
    int refresh_counter() const {
        return refresh_counter_;
    }

private:
    // -- Internal state --

    SdramState state_ = SdramState::IDLE;

    int delay_counter_ = 0;   ///< Countdown for tRCD, CL, tPRECHARGE, refresh
    int refresh_counter_ = 0; ///< Cycles since last auto-refresh

    uint32_t burst_addr_ = 0;       ///< Current word address within burst
    int burst_remaining_ = 0;       ///< Words remaining in current burst
    uint8_t burst_is_write_ = 0;    ///< Current burst is a write (not read)
    uint8_t burst_is_single_ = 0;   ///< Current access is single-word (burst_len==0)
    uint8_t single_read_phase_ = 0; ///< Phase counter for single-word 32-bit assembly

    uint16_t single_low_word_ = 0; ///< Low 16-bit word for single-word 32-bit read

    uint8_t cancel_pending_ = 0; ///< Burst cancel was requested

    /// Sparse memory storage (word_addr -> 16-bit value).
    std::unordered_map<uint32_t, uint16_t> mem_;
};
