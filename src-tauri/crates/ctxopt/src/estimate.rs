pub fn est_tokens(bytes: usize) -> usize {
    bytes / 4
}

/// Mirror of engine transcript_context::claude_model_context_window — kept
/// in sync by hand; crate purity forbids depending on the engine.
pub fn context_window_for_model(model: &str) -> usize {
    let m = model.to_ascii_lowercase();
    if m.contains("[1m]") || m.starts_with("claude-fable") || m.starts_with("claude-mythos") {
        return 1_000_000;
    }
    200_000
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn est_is_bytes_over_four() {
        assert_eq!(est_tokens(4000), 1000);
    }
    #[test]
    fn window_defaults_and_1m() {
        assert_eq!(context_window_for_model("claude-3-5-haiku-20241022"), 200_000);
        assert_eq!(context_window_for_model("claude-sonnet-5[1m]"), 1_000_000);
        assert_eq!(context_window_for_model("claude-fable-5"), 1_000_000);
    }
}
