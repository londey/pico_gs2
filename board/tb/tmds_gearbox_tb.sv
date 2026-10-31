`timescale 1ps / 1ps
`default_nettype none

// Module: tmds_gearbox_tb
// Purpose: Checks tmds_gearbox with the board's clock ratio (two clk_core
//          cycles to five clk_sclk cycles): after alignment, the four-bit
//          nibbles, joined into one serial stream per channel, carry every
//          symbol the core produced, once each, in order, on the same
//          symbol boundaries in every channel, with the TMDS clock pattern
//          on those boundaries too.
//
// Plusargs (all in ps): +sclk_offset (where the first clk_sclk edge falls),
// +reset (when rst_sclk_n is released) and +delay (clk_core to symbol
// output, like the core's clock-to-out and routing).
module tmds_gearbox_tb;

    // Clock half-periods, in ps: 13.33 ns and 5.332 ns, so that two clk_core
    // cycles last exactly five clk_sclk cycles, as on the board.
    localparam int CORE_HALF = 6665;
    localparam int SCLK_HALF = 2666;

    // clk_sclk cycles to record, after alignment.
    localparam int SAMPLES = 1000;

    // clk_sclk cycles after reset before recording starts (alignment takes
    // under ten).
    localparam int SETTLE = 40;

    // Symbols per channel to check.
    localparam int CHECKED = (SAMPLES * 4) / 10 - 2;

    // TMDS clock channel symbol, sent least significant bit first.
    localparam logic [9:0] CLOCK_SYMBOL = 10'b0000011111;

    // Inverse of 7 modulo 1024, to recover the count from the red symbol.
    localparam int INV7 = 439;

    logic        clk_core;           // Core clock
    logic        clk_sclk;           // Gearbox parallel clock
    logic        rst_sclk_n;         // Gearbox reset, released on a clk_sclk edge
    logic [9:0]  blue;               // Blue symbol: the count
    logic [9:0]  green;              // Green symbol: the count XOR 0x2AA
    logic [9:0]  red;                // Red symbol: the count times 7
    logic [15:0] nibbles;            // DUT output
    int          count;              // Symbols produced so far
    int          sclk_offset = 0;    // First clk_sclk edge, ps
    int          reset_time = 100000;// Reset release, ps
    int          delay = 1500;       // clk_core to symbol delay, ps
    bit          stream[4][$];       // Serial bits per channel: blue, green, red, clock

    tmds_gearbox dut (
        .clk_core   (clk_core),
        .clk_sclk   (clk_sclk),
        .rst_sclk_n (rst_sclk_n),
        .blue       (blue),
        .green      (green),
        .red        (red),
        .nibbles    (nibbles)
    );

    initial begin
        clk_core   = 1'b0;
        clk_sclk   = 1'b0;
        rst_sclk_n = 1'b0;
        blue       = '0;
        green      = '0;
        red        = '0;
        count      = 0;
        void'($value$plusargs("sclk_offset=%d", sclk_offset));
        void'($value$plusargs("reset=%d", reset_time));
        void'($value$plusargs("delay=%d", delay));
    end

    always begin
        #CORE_HALF clk_core = ~clk_core;
    end

    initial begin
        #(sclk_offset);
        forever begin
            #SCLK_HALF clk_sclk = ~clk_sclk;
        end
    end

    // A new symbol on each channel every clk_core cycle, `delay` after the
    // edge.
    always @(posedge clk_core) begin
        count <= count + 1;
        blue  <= #(delay) 10'(count + 1);
        green <= #(delay) 10'(count + 1) ^ 10'h2AA;
        red   <= #(delay) 10'((count + 1) * 7);
    end

    // Reset, then record the nibbles (sampled mid-cycle) and check them.
    initial begin
        #(reset_time);
        @(negedge clk_sclk);
        rst_sclk_n = 1'b1;
        repeat (SETTLE) @(posedge clk_sclk);
        repeat (SAMPLES) begin
            @(negedge clk_sclk);
            for (int ch = 0; ch < 4; ch++) begin
                for (int b = 0; b < 4; b++) begin
                    stream[ch].push_back(nibbles[4 * ch + b]);
                end
            end
        end
        check();
        $display("PASS: sclk_offset=%0d reset=%0d delay=%0d", sclk_offset, reset_time, delay);
        $finish;
    end

    // The 10-bit symbol at `start` in a channel's stream.
    function automatic logic [9:0] symbol_at(int ch, int start);
        logic [9:0] s;
        for (int b = 0; b < 10; b++) begin
            s[b] = stream[ch][start + b];
        end
        return s;
    endfunction

    // The count each channel's symbol encodes.
    function automatic int count_of(int ch, logic [9:0] s);
        case (ch)
            0: return int'(s);
            1: return int'(10'(s ^ 10'h2AA));
            default: return (int'(s) * INV7) % 1024;
        endcase
    endfunction

    task automatic check();
        int start;
        int previous;
        int now;
        // The symbol boundary: where the clock channel's pattern starts.
        start = -1;
        for (int i = 0; i < 10; i++) begin
            if (symbol_at(3, i) == CLOCK_SYMBOL) begin
                start = i;
            end
        end
        if (start < 0) begin
            $fatal(1, "no TMDS clock pattern in the clock channel");
        end
        for (int j = 0; j < CHECKED; j++) begin
            if (symbol_at(3, start + 10 * j) != CLOCK_SYMBOL) begin
                $fatal(1, "clock channel symbol %0d is %b", j, symbol_at(3, start + 10 * j));
            end
        end
        // Each data channel counts up by one per symbol, in step.
        for (int ch = 0; ch < 3; ch++) begin
            previous = count_of(ch, symbol_at(ch, start));
            if (ch > 0 && previous != count_of(0, symbol_at(0, start))) begin
                $fatal(1, "channel %0d out of step with channel 0", ch);
            end
            for (int j = 1; j < CHECKED; j++) begin
                now = count_of(ch, symbol_at(ch, start + 10 * j));
                if (now != (previous + 1) % 1024) begin
                    $fatal(1, "channel %0d symbol %0d: count %0d after %0d", ch, j, now, previous);
                end
                previous = now;
            end
        end
    endtask

endmodule

`default_nettype wire
