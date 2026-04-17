use std::fs::OpenOptions;
use std::io::prelude::*;
use std::path::Path;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

fn main() -> std::io::Result<()> {
    let path = Path::new("test-lock-racket.rkt");
    
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    
    #[cfg(windows)]
    options.share_mode(1 | 2); // FILE_SHARE_READ | FILE_SHARE_WRITE (No DELETE)

    let mut file = options.open(&path)?;
    file.write_all(b"#lang racket\n(displayln \"Racket can read me!\")\n")?;
    file.flush()?;

    println!("File created and STICKY locked. Waiting 30s for Racket test...");
    std::thread::sleep(std::time::Duration::from_secs(30));
    Ok(())
}
