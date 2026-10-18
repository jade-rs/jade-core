//! `jade-docx-perf <scenario> <fixture.docx>`
//!
//! Runs one cost scenario. Meant to be run under
//! `valgrind --tool=callgrind --collect-atstart=no --toggle-collect='*perf_measured_region*'`;
//! run plainly it prints the measured region's wall time, which is only a
//! sanity check.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [scenario, fixture] = args.as_slice() else {
        eprintln!(
            "usage: jade-docx-perf <scenario> <fixture.docx>\nscenarios: {}",
            jade_docx_perf::SCENARIOS.join(", ")
        );
        return ExitCode::from(2);
    };
    let mut fixture = PathBuf::from(fixture);
    if !fixture.exists() {
        fixture = jade_docx_perf::repo_root()
            .join("public/fixtures")
            .join(&fixture);
    }
    let started = Instant::now();
    match jade_docx_perf::run(scenario, &fixture) {
        Ok(()) => {
            eprintln!("{scenario} {}: {:?}", fixture.display(), started.elapsed());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{scenario} {}: {err}", fixture.display());
            ExitCode::FAILURE
        }
    }
}
