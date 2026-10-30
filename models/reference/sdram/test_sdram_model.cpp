// Smoke test for SdramModelSim behavioral SDRAM model.
//
// Verifies:
//   1. Write/read correctness across different SDRAM banks.
//   2. Read latency: tRCD + CL = 5 cycles from mem_req to first
//      mem_burst_data_valid.
//   3. Auto-refresh: mem_ready deasserts for >= 6 cycles every ~781 cycles.
//   4. Burst cancel: mem_ack within tPRECHARGE=2 cycles after cancel.
//   5. Single-word 32-bit read assembly.
//   6. Burst write correctness.
//
// Spec-ref: unit_037_verilator_interactive_sim.md `1a4b995821bd694a` 2026-02-28
//
// References:
//   UNIT-007 (Memory Arbiter) -- SDRAM interface and timing requirements

#include "sdram_model_sim.hpp"

#include <array>
#include <cstdio>

/// Aggregates test failure count across all test functions.
struct TestResults {
    int failures = 0;
};

/// Record a test assertion failure with source location context.
///
/// @param results   Test results accumulator.
/// @param func      Name of the calling function (__func__).
/// @param line      Source line number (__LINE__).
/// @param msg       Human-readable failure description.
inline void test_fail(TestResults& results, const char* func, int line, const char* msg) {
    std::fprintf(stderr, "FAIL: %s (line %d): %s\n", func, line, msg);
    results.failures++;
}

/// Record a test equality assertion failure with expected/actual values.
///
/// @param results   Test results accumulator.
/// @param func      Name of the calling function (__func__).
/// @param line      Source line number (__LINE__).
/// @param msg       Human-readable failure description.
/// @param expected  The expected value.
/// @param actual    The actual value.
template <typename T>
void test_fail_eq(
    TestResults& results, const char* func, int line, const char* msg, T expected, T actual
) {
    std::fprintf(
        stderr,
        "FAIL: %s (line %d): %s (expected %u, got %u)\n",
        func,
        line,
        msg,
        static_cast<unsigned>(expected),
        static_cast<unsigned>(actual)
    );
    results.failures++;
}

/// Assert a boolean condition, recording a failure if false.
///
/// Uses a macro to capture __func__ and __LINE__ at the call site.
/// A macro is used here (rather than a function) because __func__ and __LINE__
/// must be evaluated at the call site; there is no standard C++ mechanism to
/// achieve this with an inline function.
// NOLINTNEXTLINE(cppcoreguidelines-macro-usage)
#define TEST_ASSERT(results, cond, msg)                      \
    do {                                                     \
        if (!(cond)) {                                       \
            test_fail((results), __func__, __LINE__, (msg)); \
        }                                                    \
    } while (0)

/// Assert equality between two values, recording a failure with details.
///
/// A macro is used here for the same reason as TEST_ASSERT: __func__ and
/// __LINE__ must be captured at the call site.
// NOLINTNEXTLINE(cppcoreguidelines-macro-usage)
#define TEST_ASSERT_EQ(results, a, b, msg)                                \
    do {                                                                  \
        if ((a) != (b)) {                                                 \
            test_fail_eq((results), __func__, __LINE__, (msg), (b), (a)); \
        }                                                                 \
    } while (0)

/// Helper: advance the model by one cycle with no request active.
static void idle_cycle(SdramModelSim& model, uint64_t& sim_time) {
    model.mem_req = 0;
    model.mem_burst_cancel = 0;
    model.eval(sim_time++);
}

/// Helper: advance the model by N idle cycles.
///
/// A raw loop is used here because wrapping idle_cycle (which mutates both
/// model and sim_time via side effects) in std::ranges would obscure intent.
[[maybe_unused]] static void idle_cycles(SdramModelSim& model, uint64_t& sim_time, int n) {
    for (int i = 0; i < n; i++) {
        idle_cycle(model, sim_time);
    }
}

// -----------------------------------------------------------------------
// Test 1: Single-word write and read across different bank addresses
// -----------------------------------------------------------------------
static void test_single_word_write_read(TestResults& results) {
    std::printf("  test_single_word_write_read...\n");
    SdramModelSim model;
    uint64_t sim_time = 0;

    // Write to several addresses across different "banks" (different high bits).
    struct TestCase {
        uint32_t byte_addr;
        uint32_t wdata;
    };

    constexpr auto cases = std::array<TestCase, 4>{{
        {0x000000, 0xDEADBEEF}, // Bank 0, row 0
        {0x200000, 0xCAFEBABE}, // Bank 1 region
        {0x400000, 0x12345678}, // Bank 2 region
        {0x600000, 0xABCD0123}, // Bank 3 region
    }};

    for (const auto& tc : cases) {
        // Issue single-word write (burst_len=0, we=1).
        model.mem_req = 1;
        model.mem_we = 1;
        model.mem_addr = tc.byte_addr;
        model.mem_wdata = tc.wdata;
        model.mem_burst_len = 0;
        model.eval(sim_time++);

        // Wait for ack: tRCD=2 cycles then write completes.
        model.mem_req = 0;
        int max_wait = 20;
        while (!model.mem_ack && max_wait > 0) {
            model.eval(sim_time++);
            max_wait--;
        }
        TEST_ASSERT(results, model.mem_ack, "Write should ack");

        // Idle a cycle to clear state.
        idle_cycle(model, sim_time);
    }

    // Read back each address and verify.
    for (const auto& tc : cases) {
        model.mem_req = 1;
        model.mem_we = 0;
        model.mem_addr = tc.byte_addr;
        model.mem_burst_len = 0;
        model.eval(sim_time++);

        model.mem_req = 0;
        int max_wait = 20;
        while (!model.mem_ack && max_wait > 0) {
            model.eval(sim_time++);
            max_wait--;
        }
        TEST_ASSERT(results, model.mem_ack, "Read should ack");
        TEST_ASSERT_EQ(results, model.mem_rdata_32, tc.wdata, "Read data mismatch");

        idle_cycle(model, sim_time);
    }

    std::printf("  test_single_word_write_read: PASS\n");
}

// -----------------------------------------------------------------------
// Test 2: Burst read latency = tRCD + CL = 5 cycles from mem_req
// -----------------------------------------------------------------------
static void test_burst_read_latency(TestResults& results) {
    std::printf("  test_burst_read_latency...\n");
    SdramModelSim model;
    uint64_t sim_time = 0;

    // Pre-populate memory with known values using direct write_word.
    uint32_t base_word_addr = 0x1000;
    for (int i = 0; i < 8; i++) {
        model.write_word(base_word_addr + i, static_cast<uint16_t>(0xA000 + i));
    }

    // Issue burst read of 8 words.
    uint32_t byte_addr = base_word_addr * 2;
    model.mem_req = 1;
    model.mem_we = 0;
    model.mem_addr = byte_addr;
    model.mem_burst_len = 8;
    model.eval(sim_time++);
    // This is cycle 1 (the request cycle).

    model.mem_req = 0;

    // Count cycles until first mem_burst_data_valid.
    int cycles_to_first_valid = 0;
    int max_cycles = 20;
    while (!model.mem_burst_data_valid && max_cycles > 0) {
        model.eval(sim_time++);
        cycles_to_first_valid++;
        max_cycles--;
    }

    // Expected: tRCD (2) + CL (3) = 5 cycles after the request cycle.
    TEST_ASSERT_EQ(
        results,
        cycles_to_first_valid,
        SdramModelSim::TRCD + SdramModelSim::CAS_LATENCY,
        "First burst_data_valid should arrive at tRCD+CL=5 cycles"
    );

    // Verify first word value.
    TEST_ASSERT(results, model.mem_burst_data_valid, "burst_data_valid should be high");
    TEST_ASSERT_EQ(
        results, model.mem_rdata, static_cast<uint16_t>(0xA000), "First burst word mismatch"
    );

    // Read remaining 7 words (should arrive 1 per cycle).
    int words_received = 1;
    for (int i = 1; i < 8; i++) {
        model.eval(sim_time++);
        TEST_ASSERT(
            results,
            model.mem_burst_data_valid,
            "burst_data_valid should be high for each burst word"
        );
        TEST_ASSERT_EQ(
            results, model.mem_rdata, static_cast<uint16_t>(0xA000 + i), "Burst word data mismatch"
        );
        words_received++;
    }

    // The last word should have burst_done and ack.
    TEST_ASSERT(results, model.mem_burst_done, "burst_done should be asserted on last word");
    TEST_ASSERT(results, model.mem_ack, "mem_ack should be asserted on last word");
    TEST_ASSERT_EQ(results, words_received, 8, "Should receive exactly 8 burst words");

    std::printf("  test_burst_read_latency: PASS\n");
}

// -----------------------------------------------------------------------
// Test 3: Auto-refresh: mem_ready deasserts for >= 6 cycles every ~781 cycles
// -----------------------------------------------------------------------
static void test_auto_refresh(TestResults& results) {
    std::printf("  test_auto_refresh...\n");
    SdramModelSim model;
    uint64_t sim_time = 0;

    // Run idle cycles until we see mem_ready deassert.
    int cycles_run = 0;
    int max_run = SdramModelSim::REFRESH_INTERVAL + 100;
    bool saw_ready_deassert = false;
    int ready_deassert_cycle = 0;

    while (cycles_run < max_run) {
        idle_cycle(model, sim_time);
        cycles_run++;

        if (!model.mem_ready) {
            saw_ready_deassert = true;
            ready_deassert_cycle = cycles_run;
            break;
        }
    }

    TEST_ASSERT(results, saw_ready_deassert, "mem_ready should deassert for auto-refresh");
    // The refresh should happen around cycle 781.
    TEST_ASSERT(
        results,
        ready_deassert_cycle <= SdramModelSim::REFRESH_INTERVAL + 5,
        "Refresh should happen near the refresh interval"
    );

    // Count how many cycles mem_ready stays deasserted.
    int deassert_duration = 0;
    while (!model.mem_ready && deassert_duration < 100) {
        idle_cycle(model, sim_time);
        deassert_duration++;
    }

    // mem_ready should be deasserted for at least REFRESH_DURATION cycles.
    // Note: deassert_duration counts cycles after the first deassert cycle,
    // so the total deassert time is deassert_duration + 1 (the cycle we
    // detected it), but the model decrements on each eval, so we check
    // that it was deasserted for at least REFRESH_DURATION - 1 additional cycles
    // (since the first deassert cycle counts as 1).
    TEST_ASSERT(
        results,
        deassert_duration >= SdramModelSim::REFRESH_DURATION - 1,
        "mem_ready should be deasserted for at least 6 cycles"
    );

    // mem_ready should be back to 1 now.
    TEST_ASSERT(results, model.mem_ready, "mem_ready should reassert after refresh");

    std::printf("  test_auto_refresh: PASS\n");
}

// -----------------------------------------------------------------------
// Test 4: Burst cancel: mem_ack within 3 cycles of mem_burst_cancel
//         (current word + PRECHARGE = tPRECHARGE=2 cycles)
// -----------------------------------------------------------------------
static void test_burst_cancel(TestResults& results) {
    std::printf("  test_burst_cancel...\n");
    SdramModelSim model;
    uint64_t sim_time = 0;

    // Pre-populate memory.
    uint32_t base_word_addr = 0x2000;
    for (int i = 0; i < 16; i++) {
        model.write_word(base_word_addr + i, static_cast<uint16_t>(0xB000 + i));
    }

    // Issue burst read of 16 words.
    uint32_t byte_addr = base_word_addr * 2;
    model.mem_req = 1;
    model.mem_we = 0;
    model.mem_addr = byte_addr;
    model.mem_burst_len = 16;
    model.eval(sim_time++);
    model.mem_req = 0;

    // Wait for first burst_data_valid (tRCD + CL = 5 cycles).
    int max_wait = 20;
    while (!model.mem_burst_data_valid && max_wait > 0) {
        model.eval(sim_time++);
        max_wait--;
    }
    TEST_ASSERT(results, model.mem_burst_data_valid, "Should get first burst word");

    // Receive 3 more words (total 4 words received).
    int words_before_cancel = 1;
    for (int i = 0; i < 3; i++) {
        model.eval(sim_time++);
        if (model.mem_burst_data_valid) {
            words_before_cancel++;
        }
    }
    TEST_ASSERT_EQ(results, words_before_cancel, 4, "Should receive 4 words before cancel");

    // Assert burst cancel.
    model.mem_burst_cancel = 1;
    model.eval(sim_time++);
    model.mem_burst_cancel = 0;

    // Count cycles until mem_ack.
    int cycles_to_ack = 1; // The cancel cycle counts as 1.
    max_wait = 10;
    while (!model.mem_ack && max_wait > 0) {
        model.eval(sim_time++);
        cycles_to_ack++;
        max_wait--;
    }

    TEST_ASSERT(results, model.mem_ack, "mem_ack should assert after burst cancel + PRECHARGE");
    // Cancel handling: the cancel is seen in READ_BURST, which transitions
    // to PRECHARGE with tPRECHARGE=2 delay. So ack arrives after 2+1=3
    // cycles from cancel assertion (cancel cycle -> PRECHARGE countdown ->
    // ack). We allow up to 3 cycles.
    TEST_ASSERT(
        results, cycles_to_ack <= 3, "mem_ack should arrive within 3 cycles of burst_cancel"
    );

    std::printf("  test_burst_cancel: PASS\n");
}

// -----------------------------------------------------------------------
// Test 5: read_word32 helper for framebuffer readback
// -----------------------------------------------------------------------
static void test_read_word32(TestResults& results) {
    std::printf("  test_read_word32...\n");
    SdramModelSim model;

    // Write a 32-bit value as two consecutive 16-bit words.
    uint32_t word_addr = 0x5000;
    model.write_word(word_addr, 0xBEEF);     // low word
    model.write_word(word_addr + 1, 0xDEAD); // high word

    uint32_t byte_addr = word_addr * 2;
    uint32_t result = model.read_word32(byte_addr);
    TEST_ASSERT_EQ(
        results, result, 0xDEADBEEFU, "read_word32 should assemble correct 32-bit value"
    );

    std::printf("  test_read_word32: PASS\n");
}

// -----------------------------------------------------------------------
// Test 6: Burst write correctness
// -----------------------------------------------------------------------
static void test_burst_write(TestResults& results) {
    std::printf("  test_burst_write...\n");
    SdramModelSim model;
    uint64_t sim_time = 0;

    uint32_t base_word_addr = 0x3000;
    uint32_t byte_addr = base_word_addr * 2;

    // Issue burst write of 4 words.
    model.mem_req = 1;
    model.mem_we = 1;
    model.mem_addr = byte_addr;
    model.mem_burst_len = 4;
    model.eval(sim_time++);
    model.mem_req = 0;

    // Wait for ACTIVATE (tRCD=2 cycles), then the model will start
    // requesting write data via mem_burst_wdata_req.
    int max_wait = 20;
    int words_written = 0;

    while (!model.mem_ack && max_wait > 0) {
        model.eval(sim_time++);
        max_wait--;

        if (model.mem_burst_wdata_req || model.mem_burst_done) {
            // Provide write data on the next cycle.
            model.mem_burst_wdata = static_cast<uint16_t>(0xC000 + words_written);
        }

        // Check if the model consumed the previous word.
        // The WRITE_BURST state writes mem_burst_wdata and then either
        // requests the next word or signals done.
    }

    // The WRITE_BURST state: on each cycle, it writes the current
    // mem_burst_wdata, decrements remaining, and either requests next
    // or signals done.

    // Reset and redo properly.
    model.reset();
    sim_time = 0;

    model.mem_req = 1;
    model.mem_we = 1;
    model.mem_addr = byte_addr;
    model.mem_burst_len = 4;
    model.eval(sim_time++);
    // Cycle 1: request accepted, state -> ACTIVATE.

    model.mem_req = 0;

    // tRCD=2 cycles: ACTIVATE countdown.
    model.eval(sim_time++); // Cycle 2: delay_counter 2->1
    model.eval(sim_time++); // Cycle 3: delay_counter 1->0, enter WRITE_BURST,
                            //          first mem_burst_wdata_req asserted.

    // The model should now request write data.
    TEST_ASSERT(
        results,
        model.mem_burst_wdata_req,
        "First mem_burst_wdata_req should be asserted after tRCD"
    );

    // Provide data for each requested word.
    words_written = 0;
    max_wait = 20;
    while (!model.mem_ack && max_wait > 0) {
        // Provide write data.
        model.mem_burst_wdata = static_cast<uint16_t>(0xC000 + words_written);
        words_written++;
        model.eval(sim_time++);
        max_wait--;
    }

    TEST_ASSERT(results, model.mem_ack, "Burst write should complete with ack");
    TEST_ASSERT(results, model.mem_burst_done, "Burst write should signal done");

    // Verify written data via direct read.
    for (int i = 0; i < 4; i++) {
        uint16_t val = model.read_word(base_word_addr + i);
        TEST_ASSERT_EQ(
            results, val, static_cast<uint16_t>(0xC000 + i), "Burst write data mismatch"
        );
    }

    std::printf("  test_burst_write: PASS\n");
}

// -----------------------------------------------------------------------
// Test 7: Verify timing: second refresh at ~2*781 cycles
// -----------------------------------------------------------------------
static void test_refresh_periodicity(TestResults& results) {
    std::printf("  test_refresh_periodicity...\n");
    SdramModelSim model;
    uint64_t sim_time = 0;

    // Run past first refresh.
    int total_cycles = 0;
    int first_refresh_at = -1;
    int second_refresh_at = -1;
    int refreshes_seen = 0;

    int max_cycles = SdramModelSim::REFRESH_INTERVAL * 3;
    // Raw loop: this state machine tracks multiple refresh events with break
    // conditions that do not map to a standard algorithm.
    while (total_cycles < max_cycles) {
        idle_cycle(model, sim_time);
        total_cycles++;

        if (!model.mem_ready) {
            refreshes_seen++;
            if (refreshes_seen == 1) {
                first_refresh_at = total_cycles;
            } else if (refreshes_seen == 2) {
                second_refresh_at = total_cycles;
                break;
            }
            // Skip through the refresh duration.
            while (!model.mem_ready && total_cycles < max_cycles) {
                idle_cycle(model, sim_time);
                total_cycles++;
            }
        }
    }

    TEST_ASSERT(results, refreshes_seen >= 2, "Should see at least 2 refreshes");
    if (first_refresh_at >= 0 && second_refresh_at >= 0) {
        int interval = second_refresh_at - first_refresh_at;
        // The interval should be approximately REFRESH_INTERVAL + REFRESH_DURATION
        // (because the counter runs during refresh too).
        TEST_ASSERT(
            results,
            interval >= SdramModelSim::REFRESH_INTERVAL - 10,
            "Refresh interval should be approximately 781 cycles"
        );
        TEST_ASSERT(
            results,
            interval <= SdramModelSim::REFRESH_INTERVAL + SdramModelSim::REFRESH_DURATION + 10,
            "Refresh interval should not exceed expected range"
        );
    }

    std::printf("  test_refresh_periodicity: PASS\n");
}

// -----------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------
int main() {
    std::printf("Running SdramModelSim smoke tests...\n\n");

    TestResults results;

    test_single_word_write_read(results);
    test_burst_read_latency(results);
    test_auto_refresh(results);
    test_burst_cancel(results);
    test_read_word32(results);
    test_burst_write(results);
    test_refresh_periodicity(results);

    std::printf("\n");
    if (results.failures == 0) {
        std::printf("All tests PASSED.\n");
        return 0;
    }
    std::printf("%d test(s) FAILED.\n", results.failures);
    return 1;
}
