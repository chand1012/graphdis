//! Host-only configuration validation; run with --no-default-features and a host target.

use graphdis::config::{Config, MAX_CONFIG_BYTES};
use std::{fs::File, io::Read, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/example.toml"));
    let mut bytes = Vec::new();
    let read = File::open(path).and_then(|file| {
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
    });
    if let Err(error) = read {
        eprintln!("Cannot read configuration: {error}");
        return ExitCode::FAILURE;
    }
    if bytes.len() > MAX_CONFIG_BYTES {
        eprintln!("Configuration exceeds {MAX_CONFIG_BYTES} bytes");
        return ExitCode::FAILURE;
    }
    let Ok(document) = std::str::from_utf8(&bytes) else {
        eprintln!("Configuration must be UTF-8");
        return ExitCode::FAILURE;
    };
    match Config::parse(document) {
        Ok(config) => {
            println!(
                "Valid: {} sources, {} screens, {} widgets, {} history points, {} reserved series",
                config.sources().len(),
                config.screens().len(),
                config
                    .screens()
                    .iter()
                    .map(|screen| screen.widgets.len())
                    .sum::<usize>(),
                config.history_points(),
                config.tracked_series()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
