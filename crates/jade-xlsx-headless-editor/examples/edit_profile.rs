//! Times what a large workbook costs: reading it, opening it for editing,
//! typing into one cell, undoing and saving. Reports CPU time as well as
//! wall time, since wall time on a loaded machine says little.
//! Usage: cargo run --release --example edit_profile -- book.xlsx [edits] [cell]
use std::time::{Duration, Instant};

use jade_xlsx_headless_api::CellInput;
use jade_xlsx_headless_editor::HeadlessWorkbook;

/// User plus system CPU time this process has used.
fn cpu_time() -> Duration {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills the struct it is given.
    let usage = unsafe {
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
        usage.assume_init()
    };
    let time = |value: libc::timeval| {
        Duration::from_secs(value.tv_sec as u64) + Duration::from_micros(value.tv_usec as u64)
    };
    time(usage.ru_utime) + time(usage.ru_stime)
}

fn timed<T>(label: &str, body: impl FnOnce() -> T) -> T {
    let (wall, cpu) = (Instant::now(), cpu_time());
    let result = body();
    println!(
        "{label:<17} cpu {:>10.1?}  wall {:>10.1?}",
        cpu_time() - cpu,
        wall.elapsed()
    );
    result
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: edit_profile book.xlsx [edits]")?;
    let edits: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(5);
    let cell = args.next().unwrap_or_else(|| "B2".to_string());
    let bytes = std::fs::read(&path)?;
    let book = timed("read_xlsx", || jade_xlsx_reader::read_xlsx(&bytes))?;
    let sheet = book.sheets[0].name.clone();
    let mut workbook = timed("open_for_editing", || {
        HeadlessWorkbook::open_for_editing(&bytes)
    })?;
    timed("recalculate", || workbook.recalculate())?;
    for edit in 0..edits {
        timed(&format!("set {cell}"), || {
            workbook.set(&sheet, &cell, CellInput::Number(edit as f64))
        })?;
    }
    timed("undo", || workbook.undo())?;
    let saved = timed("save_bytes", || workbook.save_bytes())?;
    println!("saved {} bytes", saved.len());
    Ok(())
}
