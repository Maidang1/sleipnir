//! Selection / git-diff payloads and pipe-command argv. Pure so tests do not need GPUI.

/// Terminal selection to paste or pipe. All-whitespace is not a payload.
pub fn selection_payload(selected: &str) -> Option<String> {
    if selected.trim().is_empty() {
        None
    } else {
        Some(selected.to_string())
    }
}

/// Split `template` into argv, substituting or appending `payload` as one element.
///
/// `{}` in the template is replaced with `payload` and is not split. Without
/// `{}`, `payload` is appended. Double-quoted segments stay one argument.
pub fn format_pipe_command(template: &str, payload: &str) -> Result<Vec<String>, String> {
    if template.trim().is_empty() {
        return Err("empty command template".into());
    }

    let mut argv = split_quoted(template);
    if argv.is_empty() {
        return Err("empty command template".into());
    }

    if template.contains("{}") {
        for arg in &mut argv {
            if arg.contains("{}") {
                *arg = arg.replace("{}", payload);
            }
        }
    } else {
        argv.push(payload.to_string());
    }
    Ok(argv)
}

/// Wrap a git diff for an agent review prompt. All-whitespace is not a payload.
pub fn git_diff_payload(diff: &str) -> Option<String> {
    if diff.trim().is_empty() {
        None
    } else {
        Some(format!("Review this git diff:\n\n```\n{diff}\n```\n"))
    }
}

/// Whitespace-split, keeping double-quoted segments as one token. Quotes are dropped.
fn split_quoted(template: &str) -> Vec<String> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;

    for c in template.chars() {
        match c {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !current.is_empty() {
                    argv.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        argv.push(current);
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_selection_is_none() {
        assert_eq!(selection_payload(""), None);
        assert_eq!(selection_payload("   "), None);
        assert_eq!(selection_payload("\n\t"), None);
    }

    #[test]
    fn selection_payload_is_verbatim() {
        let selected = "  keep leading\nand trailing  \n";
        assert_eq!(
            selection_payload(selected).as_deref(),
            Some(selected),
            "non-empty after trim must keep the original bytes, including newlines"
        );
    }

    #[test]
    fn substitutes_braces_as_one_argv() {
        assert_eq!(
            format_pipe_command("review {}", "a b").unwrap(),
            vec!["review".to_string(), "a b".to_string()]
        );
    }

    #[test]
    fn splits_quoted_template_and_appends_payload() {
        assert_eq!(
            format_pipe_command(r#"sh -c "cat > /tmp/x""#, "payload").unwrap(),
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "cat > /tmp/x".to_string(),
                "payload".to_string(),
            ]
        );
    }

    #[test]
    fn empty_template_is_error() {
        assert!(format_pipe_command("", "x").is_err());
        assert!(format_pipe_command("   ", "x").is_err());
    }

    #[test]
    fn empty_diff_is_none() {
        assert_eq!(git_diff_payload(""), None);
        assert_eq!(git_diff_payload("  \n"), None);
    }

    #[test]
    fn wraps_git_diff_in_review_fence() {
        assert_eq!(
            git_diff_payload("diff --git a/x b/x").as_deref(),
            Some("Review this git diff:\n\n```\ndiff --git a/x b/x\n```\n")
        );
    }
}
