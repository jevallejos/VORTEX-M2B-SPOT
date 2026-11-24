use std::fmt;
use std::time::Duration;

use serde::Serialize;

use crate::domain::error::InfraError;
use crate::domain::ports::NotifierPort;

#[derive(Clone)]
pub struct TelegramNotifier {
    agent: ureq::Agent,
    bot_token: String,
    chat_id: String,
    base_url: String,
}

impl fmt::Debug for TelegramNotifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelegramNotifier")
            .field(
                "bot_token",
                &if self.bot_token.is_empty() {
                    ""
                } else {
                    "[REDACTED]"
                },
            )
            .field("chat_id", &self.chat_id)
            .field("base_url", &self.base_url)
            .finish()
    }
}

#[derive(Serialize)]
struct SendMessageBody<'a> {
    chat_id: &'a str,
    text: &'a str,
}

impl TelegramNotifier {
    pub fn new(bot_token: impl Into<String>, chat_id: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(5))
            .build();
        Self {
            agent,
            bot_token: bot_token.into().trim().to_string(),
            chat_id: chat_id.into().trim().to_string(),
            base_url: "https://api.telegram.org".to_string(),
        }
    }
}

impl NotifierPort for TelegramNotifier {
    fn send_message(&self, text: &str) -> Result<(), InfraError> {
        if self.bot_token.is_empty() || self.chat_id.is_empty() {
            return Ok(());
        }

        let url = format!("{}/bot{}/sendMessage", self.base_url, self.bot_token);
        let body = SendMessageBody {
            chat_id: &self.chat_id,
            text,
        };
        let payload = serde_json::to_string(&body)
            .map_err(|e| InfraError::Notifier(format!("failed to serialize telegram body: {e}")))?;

        match self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&payload)
        {
            Ok(resp) => {
                let status = resp.status();
                if (200..300).contains(&status) {
                    Ok(())
                } else {
                    Err(InfraError::Notifier(format!(
                        "telegram unexpected HTTP status {status}"
                    )))
                }
            }
            Err(ureq::Error::Status(code, resp)) => {
                let msg = resp.into_string().unwrap_or_default();
                Err(InfraError::Notifier(format!("telegram HTTP {code}: {msg}")))
            }
            Err(ureq::Error::Transport(t)) => Err(InfraError::Notifier(format!(
                "telegram transport error: {t}"
            ))),
        }
    }
}
