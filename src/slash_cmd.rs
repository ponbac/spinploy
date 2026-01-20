use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    Preview,
    Delete,
}

impl FromStr for SlashCommand {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "/preview" => Ok(SlashCommand::Preview),
            "/delete" => Ok(SlashCommand::Delete),
            _ => Err(anyhow::anyhow!("Invalid slash command: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_preview_command() {
        assert_eq!(SlashCommand::from_str("/preview").unwrap(), SlashCommand::Preview);
        assert_eq!(SlashCommand::from_str("/PREVIEW").unwrap(), SlashCommand::Preview);
    }

    #[test]
    fn parse_delete_command() {
        assert_eq!(SlashCommand::from_str("/delete").unwrap(), SlashCommand::Delete);
        assert_eq!(SlashCommand::from_str("/DELETE").unwrap(), SlashCommand::Delete);
    }

    #[test]
    fn parse_command_with_whitespace() {
        assert_eq!(SlashCommand::from_str("/preview\n").unwrap(), SlashCommand::Preview);
        assert_eq!(SlashCommand::from_str("/preview  ").unwrap(), SlashCommand::Preview);
        assert_eq!(SlashCommand::from_str("  /preview").unwrap(), SlashCommand::Preview);
        assert_eq!(SlashCommand::from_str("\n/delete\n").unwrap(), SlashCommand::Delete);
    }

    #[test]
    fn invalid_command() {
        assert!(SlashCommand::from_str("/unknown").is_err());
        assert!(SlashCommand::from_str("preview").is_err());
    }
}
