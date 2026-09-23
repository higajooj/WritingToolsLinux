//! Prompt construction for configured options.

use crate::options::OptionEntry;

pub const CUSTOM: &str = "Custom";

/// System instruction used for follow-up questions in the response window.
pub const FOLLOWUP_SYSTEM_INSTRUCTION: &str = "You are a helpful AI assistant. Provide clear and direct responses, maintaining the same format and style as your previous responses. If appropriate, use Markdown formatting to make your response more readable.";

/// Build the user prompt for a configured option and one invocation.
pub fn build_option_prompt(name: &str, option: &OptionEntry, selected_text: &str, extra: Option<&str>) -> String {
    let prefix = &option.prefix;
    let extra = extra.map(str::trim).unwrap_or_default();
    if name == CUSTOM {
        return format!("{prefix}Described change: {extra}\n\nText: {selected_text}");
    }
    if !extra.is_empty() {
        return format!("{prefix}Additional instructions: {extra}\n\nText:\n{selected_text}");
    }
    format!("{prefix}{selected_text}")
}

/// Build the system instruction for one invocation. Built-in options carry
/// standing rules ("Output ONLY...", "Respond in the same language...") that
/// would otherwise outrank per-run instructions sent only in the user turn, so
/// repeat them here with explicit precedence. Custom already treats the
/// user's text as the change.
pub fn build_system_instruction(name: &str, option: &OptionEntry, extra: Option<&str>) -> String {
    let extra = extra.map(str::trim).unwrap_or_default();
    if name == CUSTOM || extra.is_empty() {
        return option.instruction.clone();
    }
    format!(
        "{}\n\nFor this request the user also gave these instructions. Follow them, even where they override the rules above:\n{extra}",
        option.instruction
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(prefix: &str) -> OptionEntry {
        OptionEntry { prefix: prefix.into(), instruction: "Rules.".into(), ..Default::default() }
    }

    #[test]
    fn plain_option() {
        let option = entry("Proofread this:\n\n");
        assert_eq!(build_option_prompt("Proofread", &option, "hi", None), "Proofread this:\n\nhi");
        assert_eq!(build_option_prompt("Proofread", &option, "hi", Some("  ")), "Proofread this:\n\nhi");
        assert_eq!(build_system_instruction("Proofread", &option, Some(" ")), "Rules.");
    }

    #[test]
    fn option_with_extra_instructions() {
        let option = entry("Rewrite this:\n\n");
        assert_eq!(
            build_option_prompt("Rewrite", &option, "hi", Some(" be brief ")),
            "Rewrite this:\n\nAdditional instructions: be brief\n\nText:\nhi"
        );
        assert_eq!(
            build_system_instruction("Rewrite", &option, Some("be brief")),
            "Rules.\n\nFor this request the user also gave these instructions. Follow them, even where they override the rules above:\nbe brief"
        );
    }

    #[test]
    fn custom_option() {
        let option = entry("Make this change to the following text:\n\n");
        assert_eq!(
            build_option_prompt(CUSTOM, &option, "hi", Some("shout")),
            "Make this change to the following text:\n\nDescribed change: shout\n\nText: hi"
        );
        assert_eq!(build_system_instruction(CUSTOM, &option, Some("shout")), "Rules.");
    }
}
