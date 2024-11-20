use clap::Parser;
use kube::Client;
use std::error::Error;

use crate::settings::Settings;

mod reconciler;
mod settings;
mod tainter;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to TOML file from which configuration is read.
    #[arg(short, long)]
    config_file: String,
}

// Adding the actix_web::main attribute also implicitly adds tokio::main.
// See https://stackoverflow.com/a/66419283.
#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    println!(
        "Reading configuration from file at path {}",
        args.config_file
    );
    let settings = Settings::new(args.config_file.as_str())?;

    tracing_subscriber::fmt()
        .json()
        .with_max_level(settings.log.max_level)
        .with_current_span(false)
        .init();

    tracing::info!("Initializing Kubernetes client");

    let client = Client::try_default().await?;

    let tainter = tainter::Tainter::new(settings, client);

    tainter.start().await?;

    Ok(())
}

// TODO description on https://hub.docker.com/r/lassehels/tainter.
// TODO multi-arch build.
