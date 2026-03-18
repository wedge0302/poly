use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Serialize;

use crate::config::RuntimeConfig;

#[derive(Debug, Clone)]
pub struct TelegramNotifier {
    enabled: bool,
    token: String,
    chat_id: String,
    client: Client,
}

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    chat_id: &'a str,
    text: &'a str,
    disable_web_page_preview: bool,
}

impl TelegramNotifier {
    pub fn from_config(cfg: &RuntimeConfig) -> Result<Self> {
        let token = cfg.telegram_bot_token.clone().unwrap_or_default();
        let chat_id = cfg.telegram_chat_id.clone().unwrap_or_default();

        if cfg.telegram_notify && (token.is_empty() || chat_id.is_empty()) {
            bail!("telegram_notify enabled but missing bot token or chat id");
        }

        Ok(Self {
            enabled: cfg.telegram_notify,
            token,
            chat_id,
            client: Client::new(),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub async fn send(&self, text: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        self.client
            .post(url)
            .json(&SendMessageRequest {
                chat_id: &self.chat_id,
                text,
                disable_web_page_preview: true,
            })
            .send()
            .await
            .context("failed to send telegram request")?
            .error_for_status()
            .context("telegram api returned non-success status")?;

        Ok(())
    }
}
