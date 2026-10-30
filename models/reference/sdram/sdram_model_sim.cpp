// Behavioral SDRAM model implementation for the Verilator interactive simulator.
//
// Implements cycle-accurate timing for the W9825G6KH SDRAM controller
// interface consumed by UNIT-007 (Memory Arbiter):
//   - Row activation: tRCD = 2 cycles
//   - CAS latency: CL = 3 cycles
//   - PRECHARGE: tRP = 2 cycles
//   - Auto-refresh: 6-cycle blocking period every 781 cycles
//   - Burst cancel: complete current word, then PRECHARGE delay
//
// References:
//   UNIT-007 (Memory Arbiter) -- SDRAM interface specification
//   UNIT-008 (Display Controller) -- Scanline prefetch timing requirements
//   UNIT-006 (Pixel Pipeline) -- Texture cache fill timing requirements

#include "sdram_model_sim.hpp"

SdramModelSim::SdramModelSim() {
    reset();
}

void SdramModelSim::reset() {
    state_ = SdramState::IDLE;
    delay_counter_ = 0;
    refresh_counter_ = 0;
    burst_addr_ = 0;
    burst_remaining_ = 0;
    burst_is_write_ = 0;
    burst_is_single_ = 0;
    single_read_phase_ = 0;
    single_low_word_ = 0;
    cancel_pending_ = 0;

    // Default output state
    mem_rdata = 0;
    mem_rdata_32 = 0;
    mem_ack = 0;
    mem_ready = 1;
    mem_burst_data_valid = 0;
    mem_burst_wdata_req = 0;
    mem_burst_done = 0;
}

uint16_t SdramModelSim::read_word(uint32_t word_addr) const {
    if (word_addr >= TOTAL_WORDS) {
        return 0;
    }
    auto it = mem_.find(word_addr);
    if (it != mem_.end()) {
        return it->second;
    }
    return 0;
}

void SdramModelSim::write_word(uint32_t word_addr, uint16_t data) {
    if (word_addr >= TOTAL_WORDS) {
        return;
    }
    mem_[word_addr] = data;
}

uint32_t SdramModelSim::read_word32(uint32_t byte_addr) const {
    uint32_t word_addr = byte_addr / 2;
    uint16_t low = read_word(word_addr);
    uint16_t high = read_word(word_addr + 1);
    return static_cast<uint32_t>(low) | (static_cast<uint32_t>(high) << 16);
}

void SdramModelSim::eval([[maybe_unused]] uint64_t sim_time) {
    // Clear single-cycle pulse outputs at the start of each cycle.
    mem_ack = 0;
    mem_burst_data_valid = 0;
    mem_burst_wdata_req = 0;
    mem_burst_done = 0;

    // -- Auto-refresh scheduling --
    // The refresh counter runs continuously. When it reaches the interval,
    // we must perform a refresh. If we are idle, we start refresh immediately.
    // If a transfer is active, the refresh will be handled after the current
    // operation completes (the arbiter sees mem_ready deasserted and blocks
    // new grants).
    refresh_counter_++;

    bool refresh_due = (refresh_counter_ >= REFRESH_INTERVAL);

    // NOTE: The state machine below uses nested if/else logic within case
    // branches. This is intentional C++ simulation logic, not RTL style.
    // The CLAUDE.md always_ff/always_comb separation rules apply to
    // SystemVerilog RTL only, not to behavioral C++ models.
    switch (state_) {
        case SdramState::IDLE: {
            if (refresh_due) {
                // Enter refresh: deassert mem_ready for REFRESH_DURATION cycles.
                state_ = SdramState::REFRESH;
                delay_counter_ = REFRESH_DURATION;
                mem_ready = 0;
                refresh_counter_ = 0;
                break;
            }

            // Accept a new request when mem_req is asserted.
            if (mem_req && mem_ready) {
                // Convert byte address to word address.
                burst_addr_ = mem_addr / 2;

                if (mem_burst_len == 0) {
                    // Single-word mode: treat as burst length 1 for reads
                    // (actually 2 reads to assemble 32-bit), or single write.
                    burst_is_single_ = 1;
                    burst_is_write_ = mem_we;
                    burst_remaining_ = 0;
                    single_read_phase_ = 0;

                    if (mem_we) {
                        // Single-word write: go through ACTIVATE then write.
                        state_ = SdramState::ACTIVATE;
                        delay_counter_ = TRCD;
                    } else {
                        // Single-word read: ACTIVATE then CAS latency.
                        state_ = SdramState::ACTIVATE;
                        delay_counter_ = TRCD;
                    }
                } else {
                    // Burst mode.
                    burst_is_single_ = 0;
                    burst_is_write_ = mem_we;
                    burst_remaining_ = mem_burst_len;
                    cancel_pending_ = 0;

                    // Begin row activation.
                    state_ = SdramState::ACTIVATE;
                    delay_counter_ = TRCD;
                }
            }
            break;
        }

        case SdramState::ACTIVATE: {
            // Row activation delay (tRCD). mem_ready stays high during
            // activation (model is busy but the signal reflects controller
            // readiness for new requests -- during activation, we are not
            // ready for a new request, so we keep mem_ready high only in
            // the sense that the task spec says "hold mem_ready high".
            // Actually per the task: "hold mem_ready high (model is ready,
            // just not accessing yet)". We keep it high.
            delay_counter_--;
            if (delay_counter_ <= 0) {
                if (burst_is_single_) {
                    if (burst_is_write_) {
                        // Single-word write: write both 16-bit words from
                        // the 32-bit mem_wdata, then ack.
                        uint16_t low = static_cast<uint16_t>(mem_wdata & 0xFFFF);
                        uint16_t high = static_cast<uint16_t>(mem_wdata >> 16);
                        write_word(burst_addr_, low);
                        write_word(burst_addr_ + 1, high);
                        mem_ack = 1;
                        state_ = SdramState::IDLE;
                    } else {
                        // Single-word read: enter CAS latency wait.
                        state_ = SdramState::READ_CAS;
                        delay_counter_ = CAS_LATENCY;
                    }
                } else {
                    if (burst_is_write_) {
                        // Burst write: begin accepting write data.
                        state_ = SdramState::WRITE_BURST;
                        // Request the first write word immediately.
                        mem_burst_wdata_req = 1;
                    } else {
                        // Burst read: enter CAS latency wait.
                        state_ = SdramState::READ_CAS;
                        delay_counter_ = CAS_LATENCY;
                    }
                }
            }
            break;
        }

        case SdramState::READ_CAS: {
            // CAS latency countdown.
            delay_counter_--;
            if (delay_counter_ <= 0) {
                if (burst_is_single_) {
                    // Single-word read: read two consecutive 16-bit words
                    // and assemble into 32-bit result.
                    uint16_t low = read_word(burst_addr_);
                    uint16_t high = read_word(burst_addr_ + 1);
                    mem_rdata_32 = static_cast<uint32_t>(low) | (static_cast<uint32_t>(high) << 16);
                    mem_rdata = low;
                    mem_ack = 1;
                    state_ = SdramState::IDLE;
                } else {
                    // Burst read: deliver first word and transition to
                    // READ_BURST for subsequent words.
                    state_ = SdramState::READ_BURST;

                    // Deliver the first burst word.
                    mem_rdata = read_word(burst_addr_);
                    mem_burst_data_valid = 1;
                    burst_addr_++;
                    burst_remaining_--;

                    if (burst_remaining_ <= 0) {
                        // Single-word burst (burst_len=1): done immediately.
                        mem_burst_done = 1;
                        mem_ack = 1;
                        state_ = SdramState::IDLE;
                    } else if (mem_burst_cancel) {
                        // Cancel requested on the first word.
                        cancel_pending_ = 1;
                    }
                }
            }
            break;
        }

        case SdramState::READ_BURST: {
            // Check for burst cancel.
            if (mem_burst_cancel && !cancel_pending_) {
                cancel_pending_ = 1;
            }

            if (cancel_pending_) {
                // Burst cancel: complete current word (already delivered in
                // the previous cycle), enter PRECHARGE delay, then ack.
                state_ = SdramState::PRECHARGE;
                delay_counter_ = TPRECHARGE;
                cancel_pending_ = 0;
                break;
            }

            // Deliver next burst word.
            mem_rdata = read_word(burst_addr_);
            mem_burst_data_valid = 1;
            burst_addr_++;
            burst_remaining_--;

            if (burst_remaining_ <= 0) {
                // Last word of the burst.
                mem_burst_done = 1;
                mem_ack = 1;
                state_ = SdramState::IDLE;
            }
            break;
        }

        case SdramState::WRITE_BURST: {
            // Check for burst cancel.
            if (mem_burst_cancel && !cancel_pending_) {
                cancel_pending_ = 1;
            }

            if (cancel_pending_) {
                // Burst cancel: enter PRECHARGE delay, then ack.
                state_ = SdramState::PRECHARGE;
                delay_counter_ = TPRECHARGE;
                cancel_pending_ = 0;
                break;
            }

            // Write the data provided by the arbiter.
            write_word(burst_addr_, mem_burst_wdata);
            burst_addr_++;
            burst_remaining_--;

            if (burst_remaining_ <= 0) {
                // Last word of the burst.
                mem_burst_done = 1;
                mem_ack = 1;
                state_ = SdramState::IDLE;
            } else {
                // Request the next write word.
                mem_burst_wdata_req = 1;
            }
            break;
        }

        case SdramState::PRECHARGE: {
            // PRECHARGE delay after burst cancel.
            delay_counter_--;
            if (delay_counter_ <= 0) {
                mem_ack = 1;
                state_ = SdramState::IDLE;
            }
            break;
        }

        case SdramState::REFRESH: {
            // Auto-refresh: mem_ready is deasserted for REFRESH_DURATION cycles.
            delay_counter_--;
            if (delay_counter_ <= 0) {
                mem_ready = 1;
                state_ = SdramState::IDLE;
            }
            break;
        }
    }
}
