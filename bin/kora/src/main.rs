#![doc = include_str!("../README.md")]
#![doc(issue_tracker_base_url = "https://github.com/Nunchi-trade/daeji/issues/")]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod cli;

fn main() -> eyre::Result<()> {
    use clap::Parser;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    kora_cli::Backtracing::enable();
    kora_cli::SigsegvHandler::install();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    cli::Cli::parse().run()
}
