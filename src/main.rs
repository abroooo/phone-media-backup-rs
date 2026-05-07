#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

use clap::Parser;
use phone_media_backup_rs::{config, run_backup, run_clean, run_verify, Mount, RealPhone, Ssh};

use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    let indicatif_layer = IndicatifLayer::new();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")))
        .with(indicatif_layer)
        .init();
    let args = config::Args::parse();
    let cfg: config::MyConfig = confy::load_path(std::path::PathBuf::from(&args.config))
        .expect("Failed to load configuration");

    let sess = Ssh::new(&cfg)?;
    let _mount = Mount::new(&cfg)?;
    let phone = RealPhone::new(
        &sess,
        cfg.source
            .path
            .to_str()
            .expect("source.path validated ASCII at config load"),
    );

    let cmd = args
        .command
        .unwrap_or_else(|| config::Commands::Backup(config::BackupArgs::default()));
    match cmd {
        config::Commands::Backup(b) => {
            run_backup(&cfg, &phone)?;
            if b.clean {
                run_clean(&cfg, &phone, args.dry_run)?;
            }
        }
        config::Commands::Verify => run_verify(&cfg, &phone)?,
        config::Commands::Clean => run_clean(&cfg, &phone, args.dry_run)?,
    }
    Ok(())
}
