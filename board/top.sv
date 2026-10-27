`default_nettype none

// Module: top
// Purpose: ICEpi Zero board top level. Owns everything Sinter can't express
//          (clocking, reset synchronisation, I/O) and instantiates the
//          Sinter-generated GPU core.
//
// Placeholder until board bring-up: the core runs on the 50 MHz oscillator
// and drives the LEDs from a counter (about 0.75 Hz on led[0]).
module top (
    input  wire        clk,      // 50 MHz board oscillator
    input  wire  [0:0] button,   // button[0]: active-low reset push button
    output logic [4:0] led       // User LEDs, active high
);

    logic [1:0] rst_sync;       // Reset synchroniser flip-flops
    logic [1:0] rst_sync_next;  // Next synchroniser value
    logic       rst_n;          // Synchronised active-low reset for the core

    // Reset: asynchronous assert from the button, synchronous release.
    always_comb begin
        rst_sync_next = {rst_sync[0], 1'b1};
    end

    always_ff @(posedge clk or negedge button[0]) begin
        if (!button[0]) begin
            rst_sync <= 2'b00;
        end else begin
            rst_sync <= rst_sync_next;
        end
    end

    assign rst_n = rst_sync[1];

    Gpu u_gpu (
        .clk     (clk),
        .rst_n   (rst_n),
        .out_led (led)
    );

endmodule

`default_nettype wire
