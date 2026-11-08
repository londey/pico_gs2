//! Writes the setup engine's micro-ROMs, seed table and schedule report,
//! and the built-in scene's ROM.
//!
//! Usage: `setup-gen <output directory>`.

use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let dir: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or("usage: setup-gen <output directory>")?
        .into();
    fs::create_dir_all(&dir)?;
    let g = setup_model::emit::generate();
    fs::write(dir.join("setup_mul.hex"), &g.mul_hex)?;
    fs::write(dir.join("setup_alu.hex"), &g.alu_hex)?;
    fs::write(dir.join("setup_seed.hex"), &g.seed_hex)?;
    fs::write(dir.join("scene_addr.hex"), &g.scene_addr_hex)?;
    fs::write(dir.join("scene_data.hex"), &g.scene_data_hex)?;
    fs::write(dir.join("setup_report.txt"), &g.report)?;
    print!("{}", g.report);
    Ok(())
}
