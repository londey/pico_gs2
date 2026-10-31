`default_nettype none

// Module: tmds_gearbox
// Purpose: Turns the core's TMDS symbols, one per clk_core cycle, into four
//          bits per clk_sclk cycle for the ODDRX2F output gearboxes.
//
// Two clk_core cycles last exactly five clk_sclk cycles (75 MHz and
// 187.5 MHz from one PLL), so the symbols go in pairs: every fifth clk_sclk
// cycle the gearbox captures the symbol pair (the previous clk_core cycle's
// symbol, held in `prev`, then the current one) and sends it as five
// four-bit nibbles, least significant bit first. The TMDS clock channel
// sends 0b0000011111 the same way.
//
// The capture must fall where the pair is stable: well after the clk_core
// edge that changed it, and before the next. The clocks are phase locked,
// but where the clk_sclk edges fall depends on the edge-clock divider and on
// when each domain left reset, so after reset the gearbox finds the clk_core
// edges from `core_toggle` (which flips every clk_core cycle) and fixes its
// capture phase once. The first clk_sclk edge to see a toggle comes 1.5 to
// 6.8 ns after the clk_core edge (a metastable sample just moves it one edge
// later), and the capture is one clk_sclk cycle after that edge, two
// clk_core cycles later (the same point in the pattern): 6.8 to 12.2 ns after
// a clk_core edge, and at least 1.1 ns before the next.
module tmds_gearbox (
    input  wire         clk_core,    // 75 MHz core clock: one symbol per cycle
    input  wire         clk_sclk,    // 187.5 MHz gearbox parallel clock
    input  wire         rst_sclk_n,  // Active-low reset, synchronous to clk_sclk
    input  wire  [9:0]  blue,        // Blue channel symbol, clk_core domain
    input  wire  [9:0]  green,       // Green channel symbol, clk_core domain
    input  wire  [9:0]  red,         // Red channel symbol, clk_core domain
    output logic [15:0] nibbles      // Per channel, four bits per clk_sclk cycle, first in
                                     // the lowest bit: [3:0] blue, [7:4] green, [11:8] red,
                                     // [15:12] clock
);

    // TMDS clock channel pattern: five bit times high, five low.
    localparam logic [9:0] TMDS_CLOCK_SYMBOL = 10'b0000011111;

    // Capture phase after alignment. `align_now` is set the cycle after the
    // clk_core edge is seen, two clk_sclk edges after the edge that first saw
    // it; this value then reaches bit 0 of the ring in two more cycles, and
    // the capture happens at the next edge: five edges after the first one,
    // the same point in the two-clk_core-cycle pattern.
    localparam logic [4:0] ALIGNED_PHASE = 5'b00100;

    logic [2:0][9:0]  prev;            // The previous cycle's symbols: red, green, blue
    logic [2:0][9:0]  prev_next;       // Next prev
    logic             core_toggle;     // Flips every clk_core cycle
    logic             core_toggle_next;// Next core_toggle
    logic [2:0]       toggle_sync;     // core_toggle sampled on clk_sclk: [0] first, [2] last
    logic [2:0]       toggle_sync_next;// Next toggle_sync
    logic             core_edge;       // A clk_core edge was seen (toggle_sync[2] != [1])
    logic             align_now;       // Fix the capture phase this cycle
    logic             align_now_next;  // Next align_now
    logic             aligned;         // The capture phase has been fixed
    logic             aligned_next;    // Next aligned
    logic [4:0]       phase;           // One-hot capture phase ring; captures when bit 0 is set
    logic [4:0]       phase_next;      // Next phase
    logic [3:0][19:0] pair;            // Per channel (clock, red, green, blue): the symbol pair
                                       // being sent, shifted down four bits per cycle
    logic [3:0][19:0] pair_next;       // Next pair
    logic [15:0]      nibbles_next;    // Next nibbles

    // clk_core side: the previous cycle's symbols, and a level that flips on
    // every clk_core edge.
    always_comb begin
        prev_next        = {red, green, blue};
        core_toggle_next = ~core_toggle;
    end

    always_ff @(posedge clk_core) begin
        prev        <= prev_next;
        core_toggle <= core_toggle_next;
    end

    // clk_sclk side. toggle_sync[0] may go metastable; the edge is detected
    // from the later stages. Only the alignment flags are reset: the phase
    // ring powers up all zeros, so it captures nothing until it is aligned.
    always_comb begin
        toggle_sync_next = {toggle_sync[1:0], core_toggle};
        core_edge        = toggle_sync[2] ^ toggle_sync[1];
        align_now_next   = rst_sclk_n && !aligned && !align_now && core_edge;
        aligned_next     = rst_sclk_n && (aligned || align_now);
        if (align_now) begin
            phase_next = ALIGNED_PHASE;
        end else begin
            phase_next = {phase[0], phase[4:1]};
        end
        if (phase[0]) begin
            pair_next = {
                {TMDS_CLOCK_SYMBOL, TMDS_CLOCK_SYMBOL},
                {red,   prev[2]},
                {green, prev[1]},
                {blue,  prev[0]}
            };
        end else begin
            pair_next = {
                {4'b0000, pair[3][19:4]},
                {4'b0000, pair[2][19:4]},
                {4'b0000, pair[1][19:4]},
                {4'b0000, pair[0][19:4]}
            };
        end
        nibbles_next = {pair[3][3:0], pair[2][3:0], pair[1][3:0], pair[0][3:0]};
    end

    always_ff @(posedge clk_sclk) begin
        toggle_sync <= toggle_sync_next;
        align_now   <= align_now_next;
        aligned     <= aligned_next;
        phase       <= phase_next;
        pair        <= pair_next;
        nibbles     <= nibbles_next;
    end

endmodule

`default_nettype wire
