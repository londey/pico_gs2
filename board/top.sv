`default_nettype none

// Module: top
// Purpose: ICEpi Zero board top level. Owns everything Sinter can't express
//          (the PLL, reset, DDR output cells, tristate I/O) and instantiates
//          the Sinter-generated GPU core.
//
// Clocks, from one EHXPLLL on the 50 MHz oscillator (500 MHz VCO, so all
// four outputs are phase locked):
//   clk_core   100 MHz  the Sinter GPU core, SDRAM controller and display
//   clk_pixel   25 MHz  re-times the TMDS symbols for the serialisers
//   clk_tmds   125 MHz  TMDS serialisers, two bits per cycle via ODDRX1F
//   clk_sdram  100 MHz  the SDRAM's clock: clk_core lagging by 270 degrees
//
// SDRAM: the core's SDRAM outputs all come from registers on clk_core. The
// SDRAM samples them on clk_sdram, 7.5 ns after the clk_core edge that
// launched them, which leaves time for the output delay and the chip's
// set-up time, and plenty of hold. The board's own memtest found 90 degrees
// unreliable and 150-359 degrees good; 270 is the middle of that window. Read
// data reaches the FPGA around the clk_core edge one cycle after the CAS
// latency, where the core registers it (hw/sdram.snt, READ_CAPTURE).
//
// DVI out: the core produces one 10-bit TMDS symbol per channel per pixel,
// from registers that change once every four clk_core cycles (40 ns).
// clk_pixel samples them once per pixel: the clocks come from one PLL, so
// whichever clk_core edge the symbols change on, they have settled by the
// next clk_pixel edge (at least 10 ns later), and each is sampled exactly
// once. Each symbol is then shifted out least significant bit first, two
// bits per clk_tmds cycle, so five clk_tmds cycles send one symbol. The TMDS
// clock channel sends 0b0000011111 the same way. clk_tmds is exactly 5x
// clk_pixel, so loading the shifters is an ordinary synchronous path.
module top (
    input  wire        clk,      // 50 MHz board oscillator
    output logic [4:0] led,      // User LEDs, active high
    output logic [3:0] gpdi_dp,  // TMDS to the GPDI connector: blue, green, red, clock
                                 // (LVCMOS33D: the _n pins are driven inverted)
    output logic        sdram_clk,   // SDRAM clock, 100 MHz
    output logic        sdram_cke,   // SDRAM clock enable
    output logic        sdram_csn,   // SDRAM chip select, active low
    output logic        sdram_rasn,  // SDRAM row address strobe, active low
    output logic        sdram_casn,  // SDRAM column address strobe, active low
    output logic        sdram_wen,   // SDRAM write enable, active low
    output logic [1:0]  sdram_ba,    // SDRAM bank address
    output logic [12:0] sdram_a,     // SDRAM address: row A[12:0], column A[8:0]
    output logic [1:0]  sdram_dqm,   // SDRAM byte masks: [0] DQ[7:0], [1] DQ[15:8]
    inout  wire  [15:0] sdram_dq     // SDRAM data
);

    // TMDS clock channel pattern: five bit times high, five low.
    localparam logic [9:0] TMDS_CLOCK_SYMBOL = 10'b0000011111;

    // Last clk_tmds phase of each symbol.
    localparam logic [2:0] LAST_PHASE = 3'd4;

    logic             clk_core;        // 100 MHz core clock
    logic             clk_pixel;       // 25 MHz pixel clock
    logic             clk_tmds;        // 125 MHz serialiser clock
    logic             clk_sdram;       // 100 MHz SDRAM clock, clk_core lagging 270 degrees
    logic             pll_locked;      // PLL lock indicator
    logic             rst_n;           // Reset: held until the PLL locks, asynchronous
    logic [1:0]       rst_core_sync;   // clk_core reset synchroniser flip-flops
    logic [1:0]       rst_core_next;   // Next synchroniser value
    logic             rst_core_n;      // Synchronised active-low reset, clk_core domain
    logic [9:0]       tmds_blue_core;  // Blue channel symbol, clk_core domain
    logic [9:0]       tmds_green_core; // Green channel symbol, clk_core domain
    logic [9:0]       tmds_red_core;   // Red channel symbol, clk_core domain
    logic [9:0]       tmds_blue;       // Blue channel symbol, clk_pixel domain
    logic [9:0]       tmds_green;      // Green channel symbol, clk_pixel domain
    logic [9:0]       tmds_red;        // Red channel symbol, clk_pixel domain
    logic [1:0]       rst_tmds_sync;   // clk_tmds reset synchroniser flip-flops
    logic [1:0]       rst_tmds_next;   // Next synchroniser value
    logic             rst_tmds_n;      // Synchronised active-low reset, clk_tmds domain
    logic [2:0]       phase;           // Bit pair being sent (0-4), UQ3.0
    logic [2:0]       phase_next;      // Next phase
    logic [3:0][9:0]  shift;           // Per-channel shift registers: clock, red, green, blue
    logic [3:0][9:0]  shift_next;      // Next shift register values
    logic [15:0]      sdram_dq_out;    // SDRAM write data from the core
    logic             sdram_dq_oe;     // High while the core drives sdram_dq
    logic             sdram_dq_t;      // Tristate control for the DQ buffers: high releases
    logic [15:0]      sdram_dq_in;     // SDRAM data as the pins see it

    // ------------------------------------------------------------------
    // Clocks
    // ------------------------------------------------------------------
`ifdef SYNTHESIS
    (* FREQUENCY_PIN_CLKI = "50" *)
    (* FREQUENCY_PIN_CLKOP = "125" *)
    (* FREQUENCY_PIN_CLKOS = "100" *)
    (* FREQUENCY_PIN_CLKOS2 = "25" *)
    (* FREQUENCY_PIN_CLKOS3 = "100" *)
    (* ICP_CURRENT = "12" *)
    (* LPF_RESISTOR = "8" *)
    (* MFG_ENABLE_FILTEROPAMP = "1" *)
    (* MFG_GMCREF_SEL = "2" *)
    EHXPLLL #(
        .PLLRST_ENA       ("DISABLED"),
        .INTFB_WAKE       ("DISABLED"),
        .STDBY_ENABLE     ("DISABLED"),
        .DPHASE_SOURCE    ("DISABLED"),
        .OUTDIVIDER_MUXA  ("DIVA"),
        .OUTDIVIDER_MUXB  ("DIVB"),
        .OUTDIVIDER_MUXC  ("DIVC"),
        .OUTDIVIDER_MUXD  ("DIVD"),
        .CLKI_DIV         (2),           // 25 MHz phase detector
        .CLKFB_DIV        (5),           // CLKOP = 5 x 25 MHz
        .FEEDBK_PATH      ("CLKOP"),
        .CLKOP_ENABLE     ("ENABLED"),
        .CLKOP_DIV        (4),           // 500 MHz VCO / 4 = 125 MHz
        .CLKOP_CPHASE     (1),
        .CLKOP_FPHASE     (0),
        .CLKOS_ENABLE     ("ENABLED"),
        .CLKOS_DIV        (5),           // 500 MHz / 5 = 100 MHz
        .CLKOS_CPHASE     (1),
        .CLKOS_FPHASE     (0),
        .CLKOS2_ENABLE    ("ENABLED"),
        .CLKOS2_DIV       (20),          // 500 MHz / 20 = 25 MHz
        .CLKOS2_CPHASE    (1),
        .CLKOS2_FPHASE    (0),
        .CLKOS3_ENABLE    ("ENABLED"),
        .CLKOS3_DIV       (5),           // 500 MHz / 5 = 100 MHz
        .CLKOS3_CPHASE    (4),           // CLKOS + 3.75 VCO cycles (7.5 ns,
        .CLKOS3_FPHASE    (6)            // 270 degrees): 3 whole + 6/8
    ) u_pll (
        .RST          (1'b0),
        .STDBY        (1'b0),
        .CLKI         (clk),
        .CLKOP        (clk_tmds),
        .CLKOS        (clk_core),
        .CLKOS2       (clk_pixel),
        .CLKOS3       (clk_sdram),
        .CLKFB        (clk_tmds),
        .CLKINTFB     (),
        .PHASESEL0    (1'b0),
        .PHASESEL1    (1'b0),
        .PHASEDIR     (1'b1),
        .PHASESTEP    (1'b1),
        .PHASELOADREG (1'b1),
        .PLLWAKESYNC  (1'b0),
        .ENCLKOP      (1'b0),
        .ENCLKOS      (1'b0),
        .ENCLKOS2     (1'b0),
        .ENCLKOS3     (1'b0),
        .LOCK         (pll_locked),
        .INTLOCK      (),
        .REFCLK       ()
    );
`else
    // Lint/simulation stand-in: every domain on the board clock.
    assign clk_tmds   = clk;
    assign clk_core   = clk;
    assign clk_pixel  = clk;
    assign clk_sdram  = clk;
    assign pll_locked = 1'b1;
`endif

    // ------------------------------------------------------------------
    // Reset
    // ------------------------------------------------------------------

    // The buttons aren't used: on the board brought up, button[0] (C4) read
    // low whether or not either button was pressed, which held the core in
    // reset.
    assign rst_n = pll_locked;

    // clk_core domain: asynchronous assert, synchronous release. (Sinter
    // only synchronises rst_n itself in designs with several clock domains.)
    always_comb begin
        rst_core_next = {rst_core_sync[0], 1'b1};
    end

    always_ff @(posedge clk_core or negedge rst_n) begin
        if (!rst_n) begin
            rst_core_sync <= 2'b00;
        end else begin
            rst_core_sync <= rst_core_next;
        end
    end

    assign rst_core_n = rst_core_sync[1];

    // clk_tmds domain: asynchronous assert, synchronous release.
    always_comb begin
        rst_tmds_next = {rst_tmds_sync[0], 1'b1};
    end

    always_ff @(posedge clk_tmds or negedge rst_n) begin
        if (!rst_n) begin
            rst_tmds_sync <= 2'b00;
        end else begin
            rst_tmds_sync <= rst_tmds_next;
        end
    end

    assign rst_tmds_n = rst_tmds_sync[1];

    // ------------------------------------------------------------------
    // GPU core
    // ------------------------------------------------------------------
    Gpu u_gpu (
        .clk               (clk_core),
        .rst_n             (rst_core_n),
        .sdram_dq          (sdram_dq_in),
        .tmds_blue_q       (tmds_blue_core),
        .tmds_green_q      (tmds_green_core),
        .tmds_red_q        (tmds_red_core),
        .out_led           (led),
        .out_sdram_cke     (sdram_cke),
        .out_sdram_cs_n    (sdram_csn),
        .out_sdram_ras_n   (sdram_rasn),
        .out_sdram_cas_n   (sdram_casn),
        .out_sdram_we_n    (sdram_wen),
        .out_sdram_ba      (sdram_ba),
        .out_sdram_a       (sdram_a),
        .out_sdram_dqm     (sdram_dqm),
        .out_sdram_dq_out  (sdram_dq_out),
        .out_sdram_dq_oe   (sdram_dq_oe)
    );

    // ------------------------------------------------------------------
    // SDRAM clock and data bus
    // ------------------------------------------------------------------
    assign sdram_clk  = clk_sdram;
    assign sdram_dq_t = ~sdram_dq_oe;

    for (genvar i = 0; i < 16; i++) begin : g_sdram_dq
`ifdef SYNTHESIS
        BB u_bb (
            .I (sdram_dq_out[i]),
            .T (sdram_dq_t),
            .O (sdram_dq_in[i]),
            .B (sdram_dq[i])
        );
`else
        // Lint/simulation stand-in for the bidirectional buffer.
        assign sdram_dq[i]    = sdram_dq_t ? 1'bz : sdram_dq_out[i];
        assign sdram_dq_in[i] = sdram_dq[i];
`endif
    end

    // ------------------------------------------------------------------
    // TMDS symbols, re-timed to the pixel clock
    // ------------------------------------------------------------------
    always_ff @(posedge clk_pixel) begin
        tmds_blue  <= tmds_blue_core;
        tmds_green <= tmds_green_core;
        tmds_red   <= tmds_red_core;
    end

    // ------------------------------------------------------------------
    // TMDS serialisers
    // ------------------------------------------------------------------
    always_comb begin
        if (!rst_tmds_n) begin
            phase_next = 3'd0;
            shift_next = '0;
        end else if (phase == LAST_PHASE) begin
            phase_next = 3'd0;
            shift_next = {TMDS_CLOCK_SYMBOL, tmds_red, tmds_green, tmds_blue};
        end else begin
            phase_next = phase + 3'd1;
            shift_next = {
                {2'b00, shift[3][9:2]},
                {2'b00, shift[2][9:2]},
                {2'b00, shift[1][9:2]},
                {2'b00, shift[0][9:2]}
            };
        end
    end

    always_ff @(posedge clk_tmds) begin
        phase <= phase_next;
        shift <= shift_next;
    end

    // Bit 0 goes out on the rising edge, bit 1 on the falling edge.
    for (genvar ch = 0; ch < 4; ch++) begin : g_tmds_out
`ifdef SYNTHESIS
        ODDRX1F u_oddr (
            .SCLK (clk_tmds),
            .RST  (1'b0),
            .D0   (shift[ch][0]),
            .D1   (shift[ch][1]),
            .Q    (gpdi_dp[ch])
        );
`else
        // Lint/simulation stand-in for the DDR output cell.
        assign gpdi_dp[ch] = clk_tmds ? shift[ch][0] : shift[ch][1];
`endif
    end

endmodule

`default_nettype wire
