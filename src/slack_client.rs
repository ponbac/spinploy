use anyhow::Result;
use reqwest::Client;
use slack_morphism::prelude::*;
use url::Url;

/// Lightweight Slack Incoming Webhook sender built on slack-morphism request shapes.
#[derive(Clone)]
pub struct SlackWebhookClient {
    client: Client,
    webhook_url: Url,
}

impl SlackWebhookClient {
    pub fn new(webhook_url: &str) -> Result<Self> {
        let client = Client::new();
        let webhook_url = Url::parse(webhook_url)?;

        Ok(Self { client, webhook_url })
    }

    pub async fn send_text(&self, text: impl AsRef<str>) -> Result<()> {
        let req = SlackApiPostWebhookMessageRequest::new(
            SlackMessageContent::new().with_text(text.as_ref().to_string()),
        );

        self.client
            .post(self.webhook_url.clone())
            .json(&req)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}
