mod assemble;
mod bcd;

use std::{env, fs, path::PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let command = args.next();
    let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
    match (command.as_deref().and_then(|s| s.to_str()), paths.as_slice()) {
        (Some("check"), [stock]) => {
            bcd::verify_stock(&fs::read(stock)?)?;
            println!("Stock SRS-X7 package reproduced byte for byte");
        }
        (Some("assemble"), [tree, parts, vendor, stock, owner, output]) => {
            assemble::assemble(tree, parts, vendor, stock, owner, output)?;
        }
        _ => return Err("usage: x7-image check STOCK | assemble TREE PARTS VENDOR STOCK OWNER OUTPUT".into()),
    }
    Ok(())
}
