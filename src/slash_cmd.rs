use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    Preview,
    Delete,
}

impl FromStr for SlashCommand {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "/preview" => Ok(SlashCommand::Preview),
            "/delete" => Ok(SlashCommand::Delete),
            _ => Err(anyhow::anyhow!("Invalid slash command: {}", s)),
        }
    }
}
