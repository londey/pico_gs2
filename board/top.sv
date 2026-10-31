`default_nettype none

// Module: top
// Purpose: ICEpi Zero board top level. Owns everything Sinter can't express
//          (the PLL, reset, edge clocks, DDR output gearboxes, tristate I/O)
//          and instantiates the Sinter-generated GPU core.
//
// Clocks, from one EHXPLLL on the 50 MHz oscillator (750 MHz VCO, so all
// three outputs are phase locked), and an edge-clock divider:
//   clk_core    75 MHz  the Sinter GPU core, SDRAM controller and display;
//                       also the 1280x720 pixel clock (CEA-861 is 74.25 MHz)
//   clk_sdram   75 MHz  the SDRAM's clock: clk_core lagging by 270 degrees
//   clk_eclk   375 MHz  edge clock for the ODDRX2F gearboxes (two bits per
//                       cycle: 750 Mbit/s per TMDS channel)
//   clk_sclk  187.5 MHz clk_eclk / 2 (CLKDIVF): the gearboxes' parallel side,
//                       four bits per cycle
//
// SDRAM: the core's SDRAM outputs all come from registers on clk_core. The
// SDRAM samples them on clk_sdram, 10 ns after the clk_core edge that
// launched them, which leaves time for the output delay and the chip's
// set-up time, and plenty of hold. Read data reaches the FPGA around the
// clk_core edge one cycle after the CAS latency, where the core registers it
// (hw/sdram.snt, READ_CAPTURE). The board's own memtest found 270 degrees
// good at 100 MHz, as did Phase 4.
//
// DVI out: the core produces one 10-bit TMDS symbol per channel per clk_core
// cycle. tmds_gearbox turns them into four bits per clk_sclk cycle, and an
// ODDRX2F per channel sends those at two bits per clk_eclk cycle.
module top (
    input  wire         clk,         // 50 MHz board oscillator
    output logic [4:0]  led,         // User LEDs, active high
    output logic [3:0]  gpdi_dp,     // TMDS to the GPDI connector: blue, green, red, clock
                                     // (LVCMOS33D: the _n pins are driven inverted)
    output logic        sdram_clk,   // SDRAM clock, 75 MHz
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

    logic             clk_core;        // 75 MHz core and pixel clock
    logic             clk_tmds;        // 375 MHz PLL output for the edge clock
    logic             clk_sdram;       // 75 MHz SDRAM clock, clk_core lagging 270 degrees
    logic             clk_eclk;        // 375 MHz gearbox edge clock
    logic             clk_sclk;        // 187.5 MHz gearbox parallel clock
    logic             pll_locked;      // PLL lock indicator
    logic             rst_n;           // Reset: held until the PLL locks, asynchronous
    logic [1:0]       rst_core_sync;   // clk_core reset synchroniser flip-flops
    logic [1:0]       rst_core_next;   // Next synchroniser value
    logic             rst_core_n;      // Synchronised active-low reset, clk_core domain
    logic [1:0]       rst_sclk_sync;   // clk_sclk reset synchroniser flip-flops
    logic [1:0]       rst_sclk_next;   // Next synchroniser value
    logic             rst_sclk_n;      // Synchronised active-low reset, clk_sclk domain
    logic [9:0]       tmds_blue;       // Blue channel symbol, clk_core domain
    logic [9:0]       tmds_green;      // Green channel symbol, clk_core domain
    logic [9:0]       tmds_red;        // Red channel symbol, clk_core domain
    logic [15:0]      nibbles;         // Four bits per channel per clk_sclk cycle (tmds_gearbox)
    logic [15:0]      sdram_dq_out;    // SDRAM write data from the core
    logic             sdram_dq_oe;     // High while the core drives sdram_dq
    logic             sdram_dq_t;      // Tristate control for the DQ buffers: high releases
    logic [15:0]      sdram_dq_in;     // SDRAM data as the pins see it

    // ------------------------------------------------------------------
    // Clocks
    // ------------------------------------------------------------------
`ifdef SYNTHESIS
    (* FREQUENCY_PIN_CLKI = "50" *)
    (* FREQUENCY_PIN_CLKOP = "375" *)
    (* FREQUENCY_PIN_CLKOS = "75" *)
    (* FREQUENCY_PIN_CLKOS2 = "75" *)
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
        .CLKFB_DIV        (15),          // CLKOP = 15 x 25 MHz
        .FEEDBK_PATH      ("CLKOP"),
        .CLKOP_ENABLE     ("ENABLED"),
        .CLKOP_DIV        (2),           // 750 MHz VCO / 2 = 375 MHz
        .CLKOP_CPHASE     (0),
        .CLKOP_FPHASE     (0),
        .CLKOS_ENABLE     ("ENABLED"),
        .CLKOS_DIV        (10),          // 750 MHz / 10 = 75 MHz
        .CLKOS_CPHASE     (0),
        .CLKOS_FPHASE     (0),
        .CLKOS2_ENABLE    ("ENABLED"),
        .CLKOS2_DIV       (10),          // 750 MHz / 10 = 75 MHz
        .CLKOS2_CPHASE    (7),           // CLKOS + 7.5 VCO cycles (10 ns,
        .CLKOS2_FPHASE    (4)            // 270 degrees): 7 whole + 4/8
    ) u_pll (
        .RST          (1'b0),
        .STDBY        (1'b0),
        .CLKI         (clk),
        .CLKOP        (clk_tmds),
        .CLKOS        (clk_core),
        .CLKOS2       (clk_sdram),
        .CLKOS3       (),
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

    ECLKSYNCB u_eclksync (
        .ECLKI (clk_tmds),
        .STOP  (1'b0),
        .ECLKO (clk_eclk)
    );

    CLKDIVF #(
        .DIV ("2.0")
    ) u_clkdiv (
        .CLKI    (clk_eclk),
        .RST     (1'b0),
        .ALIGNWD (1'b0),
        .CDIVX   (clk_sclk)
    );
`else
    // Lint/simulation stand-in: every domain on the board clock.
    assign clk_tmds   = clk;
    assign clk_core   = clk;
    assign clk_sdram  = clk;
    assign clk_eclk   = clk_tmds;
    assign clk_sclk   = clk_tmds;
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

    // clk_sclk domain: asynchronous assert, synchronous release.
    always_comb begin
        rst_sclk_next = {rst_sclk_sync[0], 1'b1};
    end

    always_ff @(posedge clk_sclk or negedge rst_n) begin
        if (!rst_n) begin
            rst_sclk_sync <= 2'b00;
        end else begin
            rst_sclk_sync <= rst_sclk_next;
        end
    end

    assign rst_sclk_n = rst_sclk_sync[1];

    // ------------------------------------------------------------------
    // GPU core
    // ------------------------------------------------------------------
    Gpu u_gpu (
        .clk               (clk_core),
        .rst_n             (rst_core_n),
        .sdram_dq          (sdram_dq_in),
        .tmds_blue_q       (tmds_blue),
        .tmds_green_q      (tmds_green),
        .tmds_red_q        (tmds_red),
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
    // TMDS serialisers
    // ------------------------------------------------------------------

    tmds_gearbox u_gearbox (
        .clk_core   (clk_core),
        .clk_sclk   (clk_sclk),
        .rst_sclk_n (rst_sclk_n),
        .blue       (tmds_blue),
        .green      (tmds_green),
        .red        (tmds_red),
        .nibbles    (nibbles)
    );

    // The gearboxes send D0 first, then D1, D2, D3, two bits per clk_eclk
    // cycle.
    for (genvar ch = 0; ch < 4; ch++) begin : g_tmds_out
`ifdef SYNTHESIS
        ODDRX2F u_oddr (
            .SCLK (clk_sclk),
            .ECLK (clk_eclk),
            .RST  (1'b0),
            .D0   (nibbles[4 * ch]),
            .D1   (nibbles[4 * ch + 1]),
            .D2   (nibbles[4 * ch + 2]),
            .D3   (nibbles[4 * ch + 3]),
            .Q    (gpdi_dp[ch])
        );
`else
        // Lint stand-in for the gearbox: not cycle accurate, but uses its
        // inputs.
        assign gpdi_dp[ch] = clk_eclk ? (nibbles[4 * ch] ^ nibbles[4 * ch + 1])
                                      : (nibbles[4 * ch + 2] ^ nibbles[4 * ch + 3]);
`endif
    end

endmodule

`default_nettype wire
