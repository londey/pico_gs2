`default_nettype none

// Module: top
// Purpose: ICEpi Zero board top level. Owns everything Sinter can't express
//          (the PLL, reset, DDR output cells, I/O) and instantiates the
//          Sinter-generated GPU core.
//
// Clocks, from one EHXPLLL on the 50 MHz oscillator (500 MHz VCO, so all
// three outputs are phase aligned):
//   clk_core   100 MHz  GPU core (Sinter domain `core`)
//   clk_pixel   25 MHz  video timing and TMDS encoding (Sinter domain `pixel`)
//   clk_tmds   125 MHz  TMDS serialisers, two bits per cycle via ODDRX1F
//
// DVI out: the core produces one 10-bit TMDS symbol per channel per pixel
// clock. Each symbol is shifted out least significant bit first, two bits
// per clk_tmds cycle, so five clk_tmds cycles send one symbol. The TMDS
// clock channel sends 0b0000011111 the same way. clk_tmds is exactly 5x
// clk_pixel from the same PLL, so loading the symbol registers into the
// shifters is an ordinary synchronous path.
module top (
    input  wire        clk,      // 50 MHz board oscillator
    input  wire  [0:0] button,   // button[0]: active-low reset push button
    output logic [4:0] led,      // User LEDs, active high
    output logic [3:0] gpdi_dp   // TMDS to the GPDI connector: blue, green, red, clock
                                 // (LVCMOS33D: the _n pins are driven inverted)
);

    // TMDS clock channel pattern: five bit times high, five low.
    localparam logic [9:0] TMDS_CLOCK_SYMBOL = 10'b0000011111;

    // Last clk_tmds phase of each symbol.
    localparam logic [2:0] LAST_PHASE = 3'd4;

    logic             clk_core;        // 100 MHz core clock
    logic             clk_pixel;       // 25 MHz pixel clock
    logic             clk_tmds;        // 125 MHz serialiser clock
    logic             pll_locked;      // PLL lock indicator
    logic             rst_n;           // Core reset: until lock, or while button[0] is held
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

    // ------------------------------------------------------------------
    // Clocks
    // ------------------------------------------------------------------
`ifdef SYNTHESIS
    (* FREQUENCY_PIN_CLKI = "50" *)
    (* FREQUENCY_PIN_CLKOP = "125" *)
    (* FREQUENCY_PIN_CLKOS = "100" *)
    (* FREQUENCY_PIN_CLKOS2 = "25" *)
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
        .CLKOS2_FPHASE    (0)
    ) u_pll (
        .RST          (1'b0),
        .STDBY        (1'b0),
        .CLKI         (clk),
        .CLKOP        (clk_tmds),
        .CLKOS        (clk_core),
        .CLKOS2       (clk_pixel),
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
`else
    // Lint/simulation stand-in: every domain on the board clock.
    assign clk_tmds   = clk;
    assign clk_core   = clk;
    assign clk_pixel  = clk;
    assign pll_locked = 1'b1;
`endif

    // ------------------------------------------------------------------
    // Reset
    // ------------------------------------------------------------------

    // The core synchronises rst_n into each of its domains itself.
    assign rst_n = pll_locked & button[0];

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
        .clk_core     (clk_core),
        .clk_pixel    (clk_pixel),
        .rst_n        (rst_n),
        .tmds_blue_q  (tmds_blue),
        .tmds_green_q (tmds_green),
        .tmds_red_q   (tmds_red),
        .core_out_led (led)
    );

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
