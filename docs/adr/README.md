# Architecture decision records

One file per decision, numbered in order: `NNNN-short-title.md`, with
**Status** (Proposed, Accepted, Superseded by NNNN), **Context**,
**Decision** and **Consequences**. A decision that changes gets a new record
that supersedes the old one; the old one stays.

| # | Decision | Status |
|---|---|---|
| [0001](0001-microcoded-setup-engine.md) | Microcoded triangle setup engine | Accepted |
| [0002](0002-built-in-test-scene.md) | Built-in test scene before the host link | Accepted; its packet format superseded by 0008 |
| [0003](0003-pixel-path-and-buffers.md) | Tiled 8×8 blocks, 16-bit Z, triple buffering | Accepted |
| [0004](0004-generated-files-at-build-time.md) | Generated files at build time only | Accepted |
| [0005](0005-simulator-verilator-oracle.md) | Simulator vs Verilator oracle | Accepted |
| [0006](0006-datapath-reset-stopgap.md) | Only control state is reset | Accepted |
| [0007](0007-fixed-point-notation.md) | Sinter fixed-point notation | Accepted |
| [0008](0008-register-host-interface.md) | A register host interface | Accepted |
| [0009](0009-v1-interface.md) | The v1.0 interface: textures, combiner, memory access | Accepted |
| [0010](0010-v1-1-draw-speed.md) | v1.1: drawing speed at the v1.0 interface | Accepted |
