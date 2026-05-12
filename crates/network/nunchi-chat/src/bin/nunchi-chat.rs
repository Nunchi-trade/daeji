//! Standalone Nunchi Chat client binary.

use std::path::PathBuf;

use clap::Parser;
use nunchi_chat::service::{ChatConfig, ChatServiceError};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Parser)]
#[command(name = "nunchi-chat")]
#[command(about = "Run the Nunchi Chat client swarm as a standalone process")]
struct Cli {
    /// Path to a Nunchi Chat TOML/JSON config file.
    #[arg(short, long, value_name = "FILE")]
    config: PathBuf,

    /// Disable chat at startup after loading config.
    #[arg(long)]
    disable_chat: bool,
}

impl Cli {
    fn load_config(&self) -> eyre::Result<ChatConfig> {
        let mut config = ChatConfig::load_from_path(&self.config)
            .map_err(|err| eyre::eyre!("load chat config {}: {}", self.config.display(), err))?;
        if self.disable_chat {
            config.enabled = false;
        }
        Ok(config)
    }

    fn run(self) -> eyre::Result<()> {
        let config = self.load_config()?;
        if !config.enabled {
            tracing::info!("nunchi-chat: disabled by config or --disable-chat; exiting");
            return Ok(());
        }

        nunchi_chat::service::run_chat_standalone(config).map_err(|err| match err {
            ChatServiceError::Disabled => eyre::eyre!("nunchi-chat disabled before startup"),
            other => eyre::eyre!(other),
        })
    }
}

fn main() -> eyre::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    Cli::parse().run()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use nunchi_chat::identity::MessageSignaturePolicy;

    use super::*;

    fn temp_config_path(ext: &str, body: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("nunchi-chat-bin-{nanos}.{ext}"));
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn parses_required_config_arg() {
        let cli = Cli::try_parse_from(["nunchi-chat", "--config", "chat.toml"]).unwrap();
        assert_eq!(cli.config, PathBuf::from("chat.toml"));
        assert!(!cli.disable_chat);
    }

    #[test]
    fn load_config_applies_disable_flag() {
        let path = temp_config_path(
            "json",
            r#"{
                "enabled": true,
                "me_seed": 1,
                "bind_port": 4101,
                "registry_path": "/tmp/nunchi-chat-registry.json"
            }"#,
        );
        let cli = Cli { config: path.clone(), disable_chat: true };

        let config = cli.load_config().unwrap();

        assert!(!config.enabled);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn load_config_preserves_signature_policy() {
        let path = temp_config_path(
            "toml",
            r#"
enabled = true
me_seed = 7
bind_port = 4107
registry_path = "/tmp/nunchi-chat-registry.json"
message_signature_policy = "local-testnet-disabled"
"#,
        );
        let cli = Cli { config: path.clone(), disable_chat: false };

        let config = cli.load_config().unwrap();

        assert_eq!(config.message_signature_policy, MessageSignaturePolicy::LocalTestnetDisabled);
        fs::remove_file(path).unwrap();
    }
}
