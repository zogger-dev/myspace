use std::path::Path;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

pub fn init_logging(log_dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, "myspace.log");

    // We only log to the file so it doesn't mess with TUI/Console stdout
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("myspace=debug".parse().unwrap()),
        )
        .with_writer(non_blocking)
        .with_ansi(false) // No colors in the log file
        .init();

    guard
}
