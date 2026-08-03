use crate::cli::{LogFormat, LogLevel};
use anyhow::{Context, Result};
use std::io::IsTerminal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init(level: Option<LogLevel>, format: LogFormat) -> Result<()> {
    let filter = match level {
        Some(level) => EnvFilter::try_new(level.directive())?,
        None => match std::env::var("RUST_LOG") {
            Ok(directive) => EnvFilter::try_new(directive).context("invalid RUST_LOG filter")?,
            Err(std::env::VarError::NotPresent) => EnvFilter::try_new(
                "off,toaster=info,toaster_cpu=info,toaster_gpu=info,toaster_server=info",
            )?,
            Err(error) => return Err(error).context("RUST_LOG is not valid Unicode"),
        },
    };

    match format {
        LogFormat::Text => tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_ansi(std::io::stderr().is_terminal()),
            )
            .try_init()
            .context("failed to initialize logging"),
        LogFormat::Json => tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(std::io::stderr)
                    .with_ansi(false),
            )
            .try_init()
            .context("failed to initialize logging"),
    }
}
