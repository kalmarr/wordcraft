//! Compress the pinned dictionary at build time; preserve upstream source bytes on disk.
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    for name in ["hu_HU.aff", "hu_HU.dic"] {
        let source = PathBuf::from("../../assets/spelling/hu").join(name);
        println!("cargo:rerun-if-changed={}", source.display());
        let bytes = fs::read(source)?;
        if bytes.len() > 16 << 20 {
            return Err("Hungarian dictionary exceeds 16 MiB per file".into());
        }
        fs::write(out.join(format!("{name}.deflate")), miniz_oxide::deflate::compress_to_vec(&bytes, 6))?;
    }
    Ok(())
}
