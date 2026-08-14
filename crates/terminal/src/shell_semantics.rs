//! Pure shell-integration helpers: OSC 133 inject scripts, click-to-move,
//! and command-output ranges. Wired from spawn / `mouse_down`.

/// Interactive shells we can auto-inject OSC 133 A/B/C/D into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectShell {
    Zsh,
    Bash,
    Fish,
}

impl InjectShell {
    /// Detect a supported interactive shell from a program path or name.
    pub fn from_program(program: &str) -> Option<Self> {
        let name = std::path::Path::new(program)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(program);
        match name {
            "zsh" => Some(Self::Zsh),
            "bash" => Some(Self::Bash),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }
}

use collections::HashMap;
use std::path::{Path, PathBuf};

/// Script text sourced into a newly spawned interactive shell.
pub fn inject_script(shell: InjectShell) -> &'static str {
    match shell {
        InjectShell::Zsh => ZSH_SCRIPT,
        InjectShell::Bash => BASH_SCRIPT,
        InjectShell::Fish => FISH_SCRIPT,
    }
}

const ZSH_SCRIPT: &str = r#"# Sleipnir OSC 133 (A/B/C/D). Opt-in inject; skip if already present.
[[ -n ${SLEIPNIR_OSC133_LOADED:-} ]] && return
SLEIPNIR_OSC133_LOADED=1
if [[ -n ${ITERM_SESSION_ID:-} || -n ${GHOSTTY_RESOURCES_DIR:-} || -n ${KITTY_SHELL_INTEGRATION:-} || -n ${WEZTERM_EXECUTABLE:-} ]]; then
  return
fi

_sleipnir_precmd() {
  printf '\033]133;D;%s\007' "$?"
  printf '\033]133;A\007'
}
_sleipnir_preexec() {
  printf '\033]133;C\007'
}
autoload -Uz add-zsh-hook 2>/dev/null || true
add-zsh-hook precmd _sleipnir_precmd 2>/dev/null || precmd_functions+=(_sleipnir_precmd)
add-zsh-hook preexec _sleipnir_preexec 2>/dev/null || preexec_functions+=(_sleipnir_preexec)
# B after the prompt so the recorded column is the input start, not 0.
# Literal OSC: stock zsh has prompt_subst off, so $(printf ...) is printed.
PS1="$PS1"$'%{\e]133;B\a%}'
"#;

const BASH_SCRIPT: &str = r#"# Sleipnir OSC 133 (A/B/C/D). Opt-in inject; skip if already present.
[ -n "${SLEIPNIR_OSC133_LOADED:-}" ] && return
SLEIPNIR_OSC133_LOADED=1
if [ -n "${ITERM_SESSION_ID:-}" ] || [ -n "${GHOSTTY_RESOURCES_DIR:-}" ] || [ -n "${KITTY_SHELL_INTEGRATION:-}" ] || [ -n "${WEZTERM_EXECUTABLE:-}" ]; then
  return
fi

_sleipnir_prompt() {
  local _s=$?
  printf '\033]133;D;%s\007' "$_s"
  printf '\033]133;A\007'
}
if [ -n "${PROMPT_COMMAND:-}" ]; then
  PROMPT_COMMAND="_sleipnir_prompt;${PROMPT_COMMAND}"
else
  PROMPT_COMMAND="_sleipnir_prompt"
fi
# B after the prompt so the recorded column is the input start, not 0.
PS1="$PS1"'\[\033]133;B\007\]'
# C: command about to run (bash 4.4+ PS0).
PS0='\033]133;C\007'
"#;

const FISH_SCRIPT: &str = r#"# Sleipnir OSC 133 (A/B/C/D). Opt-in inject; skip if already present.
if set -q SLEIPNIR_OSC133_LOADED
    return
end
set -g SLEIPNIR_OSC133_LOADED 1
if set -q ITERM_SESSION_ID; or set -q GHOSTTY_RESOURCES_DIR; or set -q KITTY_SHELL_INTEGRATION; or set -q WEZTERM_EXECUTABLE
    return
end

function __sleipnir_precmd --on-event fish_prompt
    printf '\033]133;D;%s\007' $status
    printf '\033]133;A\007'
end

function __sleipnir_preexec --on-event fish_preexec
    printf '\033]133;C\007'
end

# B after the user's prompt function so prefix_cols is the input start.
if functions -q fish_prompt
    functions -c fish_prompt __sleipnir_orig_fish_prompt
    function fish_prompt
        __sleipnir_orig_fish_prompt $argv
        printf '\033]133;B\007'
    end
else
    function fish_prompt
        printf '\033]133;B\007'
    end
end
"#;

/// Directory that holds generated inject scripts (under the user config dir).
pub fn inject_script_dir() -> PathBuf {
    sleipnir_settings::config_dir().join("shell-integration")
}

/// Wrap an interactive spawn so the inject script is sourced.
/// When `enabled` is false, or the program is not zsh/bash/fish, argv is unchanged.
pub fn wrap_shell_for_inject(
    program: &str,
    args: Option<Vec<String>>,
    env: &mut HashMap<String, String>,
    enabled: bool,
) -> (String, Option<Vec<String>>) {
    wrap_shell_for_inject_in(program, args, env, enabled, &inject_script_dir())
}

/// Same as [`wrap_shell_for_inject`] with an explicit script directory (tests).
pub fn wrap_shell_for_inject_in(
    program: &str,
    args: Option<Vec<String>>,
    env: &mut HashMap<String, String>,
    enabled: bool,
    script_dir: &Path,
) -> (String, Option<Vec<String>>) {
    if !enabled {
        return (program.to_string(), args);
    }
    if args
        .as_ref()
        .is_some_and(|a| a.iter().any(|x| x == "-c" || x == "--command"))
    {
        return (program.to_string(), args);
    }
    let Some(shell) = InjectShell::from_program(program) else {
        return (program.to_string(), args);
    };
    if let Err(err) = std::fs::create_dir_all(script_dir) {
        log::warn!(
            "osc133 inject: cannot create {}: {err}",
            script_dir.display()
        );
        return (program.to_string(), args);
    }
    env.insert("SLEIPNIR_SHELL_INTEGRATION".into(), "1".into());
    match shell {
        InjectShell::Zsh => wrap_zsh(program, args, env, script_dir),
        InjectShell::Bash => wrap_bash(program, args, env, script_dir),
        InjectShell::Fish => wrap_fish(program, args, env, script_dir),
    }
}

fn write_script(path: &Path, body: &str) {
    if let Err(err) = std::fs::write(path, body) {
        log::warn!("osc133 inject: cannot write {}: {err}", path.display());
    }
}

fn wrap_zsh(
    program: &str,
    args: Option<Vec<String>>,
    env: &mut HashMap<String, String>,
    script_dir: &Path,
) -> (String, Option<Vec<String>>) {
    let script_path = script_dir.join("osc133.zsh");
    write_script(&script_path, inject_script(InjectShell::Zsh));
    let zdot = script_dir.join("zsh");
    if let Err(err) = std::fs::create_dir_all(&zdot) {
        log::warn!("osc133 inject: cannot create {}: {err}", zdot.display());
        return (program.to_string(), args);
    }
    let zshrc = format!(
        r#"# Sleipnir ZDOTDIR wrapper: user zshrc first, then OSC 133 (so PS1 wrap sticks).
if [[ -n ${{SLEIPNIR_USER_ZDOTDIR+x}} ]]; then
  export ZDOTDIR="$SLEIPNIR_USER_ZDOTDIR"
  unset SLEIPNIR_USER_ZDOTDIR
else
  unset ZDOTDIR
fi
if [[ -n ${{ZDOTDIR:-}} && -f "$ZDOTDIR/.zshrc" ]]; then
  source "$ZDOTDIR/.zshrc"
elif [[ -f "$HOME/.zshrc" ]]; then
  source "$HOME/.zshrc"
fi
source "{script}"
"#,
        script = script_path.display()
    );
    write_script(&zdot.join(".zshrc"), &zshrc);
    if let Some(existing) = env.get("ZDOTDIR").cloned() {
        env.insert("SLEIPNIR_USER_ZDOTDIR".into(), existing);
    }
    env.insert("ZDOTDIR".into(), zdot.to_string_lossy().into_owned());
    (program.to_string(), args)
}

fn wrap_bash(
    program: &str,
    args: Option<Vec<String>>,
    _env: &mut HashMap<String, String>,
    script_dir: &Path,
) -> (String, Option<Vec<String>>) {
    let script_path = script_dir.join("osc133.bash");
    write_script(&script_path, inject_script(InjectShell::Bash));
    let rcfile = script_dir.join("bash.rc");
    let rc = format!(
        r#"# Sleipnir bash --rcfile: user bashrc first, then OSC 133 (so PS1 wrap sticks).
if [ -f /etc/bash.bashrc ]; then
  . /etc/bash.bashrc
fi
if [ -f "$HOME/.bashrc" ]; then
  . "$HOME/.bashrc"
fi
source "{script}"
"#,
        script = script_path.display()
    );
    write_script(&rcfile, &rc);
    let mut out = vec!["--rcfile".into(), rcfile.to_string_lossy().into_owned()];
    if let Some(rest) = args {
        out.extend(rest);
    }
    (program.to_string(), Some(out))
}

fn wrap_fish(
    program: &str,
    args: Option<Vec<String>>,
    _env: &mut HashMap<String, String>,
    script_dir: &Path,
) -> (String, Option<Vec<String>>) {
    let script_path = script_dir.join("osc133.fish");
    write_script(&script_path, inject_script(InjectShell::Fish));
    let init = format!("source '{}'", script_path.display());
    let mut out = vec!["-C".into(), init];
    if let Some(rest) = args {
        out.extend(rest);
    }
    (program.to_string(), Some(out))
}

/// Apply inject to a [`task_types::Shell`] for an interactive pane spawn.
pub fn apply_inject_to_shell(
    shell: task_types::Shell,
    env: &mut HashMap<String, String>,
    enabled: bool,
) -> task_types::Shell {
    use task_types::Shell;
    if !enabled {
        return shell;
    }
    let (program, args) = match &shell {
        Shell::System => (shell.program(), None),
        Shell::Program(program) => (program.clone(), None),
        Shell::WithArguments { program, args, .. } => (program.clone(), Some(args.clone())),
    };
    let title = match &shell {
        Shell::WithArguments { title_override, .. } => title_override.clone(),
        _ => None,
    };
    let (new_program, new_args) = wrap_shell_for_inject(&program, args.clone(), env, true);
    if new_program == program && new_args == args {
        return shell;
    }
    Shell::WithArguments {
        program: new_program,
        args: new_args.unwrap_or_default(),
        title_override: title,
    }
}

/// Convert an absolute scrollback line (as stored in OSC 133 markers) to an
/// alacritty grid line (viewport-relative), given the current scrollback depth.
///
/// Markers are recorded as `cursor.line + history_size` so they stay stable as
/// content scrolls; click/cursor points from the mouse layer are grid lines.
/// This is the single bridge between the two coordinate spaces.
pub fn absolute_to_grid_line(absolute_line: i32, history_size: i32) -> i32 {
    absolute_line - history_size
}

/// Inputs for Option/Alt-click cursor movement inside the current prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClickToMove {
    pub click_line: i32,
    pub click_column: usize,
    pub cursor_line: i32,
    pub cursor_column: usize,
    /// Last OSC 133 A line, if any.
    pub prompt_line: Option<i32>,
    /// Display columns occupied by the prompt prefix (not editable).
    pub prompt_prefix_cols: usize,
    pub alt_screen: bool,
}

/// CSI left/right sequence that moves the shell cursor to the clicked cell.
/// `None` means do not invent movement.
pub fn click_to_move_sequence(req: ClickToMove) -> Option<Vec<u8>> {
    if req.alt_screen {
        return None;
    }
    let prompt_line = req.prompt_line?;
    if req.click_line != req.cursor_line {
        return None;
    }
    if req.cursor_line < prompt_line || req.click_line < prompt_line {
        return None;
    }
    if req.click_column < req.prompt_prefix_cols {
        return None;
    }
    let delta = req.click_column as i32 - req.cursor_column as i32;
    if delta == 0 {
        return Some(Vec::new());
    }
    let (n, seq) = if delta > 0 {
        (delta as usize, b"\x1b[C".as_slice())
    } else {
        ((-delta) as usize, b"\x1b[D".as_slice())
    };
    Some(seq.repeat(n))
}

/// How a triple-click should select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TripleClickKind {
    /// Cmd/Ctrl-triple-click: the recorded command output.
    CommandOutput(crate::SelectionRange),
    /// Plain triple-click (or no usable markers): whole lines, as today.
    Lines,
}

/// Cmd/Ctrl-triple-click uses OSC 133 C/D (or A/B) to bound output.
/// Without the modifier, or without a containing command, this is [`TripleClickKind::Lines`].
///
/// `markers` store absolute scrollback lines (`cursor.line + history_size`),
/// while `click_line` is an alacritty grid line (viewport-relative). `history_size`
/// bridges the two so a click matches markers regardless of scrollback depth.
pub fn triple_click_kind(
    primary_mod: bool,
    markers: &[crate::Osc133Marker],
    click_line: i32,
    last_column: usize,
    history_size: i32,
) -> TripleClickKind {
    if primary_mod {
        if let Some(range) = command_output_range(markers, click_line, last_column, history_size) {
            return TripleClickKind::CommandOutput(range);
        }
    }
    TripleClickKind::Lines
}

/// Inclusive cell range of the command output that contains `click_line`.
///
/// `markers` carry absolute scrollback lines; `click_line` is a grid line.
/// The returned [`crate::SelectionRange`] is in grid coordinates, ready to hand
/// to the alacritty selection.
pub fn command_output_range(
    markers: &[crate::Osc133Marker],
    click_line: i32,
    last_column: usize,
    history_size: i32,
) -> Option<crate::SelectionRange> {
    // Compare in the markers' absolute coordinate space.
    let click_abs = click_line + history_size;
    let marked: Vec<(crate::Osc133Kind, i32)> = markers
        .iter()
        .filter_map(|m| m.line.map(|line| (m.kind, line)))
        .collect();
    if marked.is_empty() {
        return None;
    }

    let abs_range = range_from_c_d(&marked, click_abs, last_column)
        .or_else(|| range_from_prompt_pair(&marked, click_abs, last_column))?;
    // Convert the range back to grid coordinates for selection.
    Some(crate::SelectionRange {
        start: crate::Point::new(abs_range.start.line - history_size, abs_range.start.column),
        end: crate::Point::new(abs_range.end.line - history_size, abs_range.end.column),
        is_block: abs_range.is_block,
    })
}

fn range_from_c_d(
    marked: &[(crate::Osc133Kind, i32)],
    click_line: i32,
    last_column: usize,
) -> Option<crate::SelectionRange> {
    let start_idx = marked.iter().rposition(|(kind, line)| {
        *line <= click_line
            && matches!(
                kind,
                crate::Osc133Kind::CommandExecuted | crate::Osc133Kind::CommandStart
            )
    })?;
    let start_line = marked[start_idx].1;
    let end_line = marked[start_idx + 1..]
        .iter()
        .find(|(kind, line)| {
            *line >= start_line
                && matches!(
                    kind,
                    crate::Osc133Kind::CommandFinished { .. } | crate::Osc133Kind::PromptStart
                )
        })
        .map(|(_, line)| *line)?;
    if click_line < start_line || click_line > end_line {
        return None;
    }
    Some(crate::SelectionRange {
        start: crate::Point::new(start_line, 0),
        end: crate::Point::new(end_line, last_column),
        is_block: false,
    })
}

fn range_from_prompt_pair(
    marked: &[(crate::Osc133Kind, i32)],
    click_line: i32,
    last_column: usize,
) -> Option<crate::SelectionRange> {
    let starts: Vec<i32> = marked
        .iter()
        .filter_map(|(kind, line)| matches!(kind, crate::Osc133Kind::PromptStart).then_some(*line))
        .collect();
    let prev = starts
        .iter()
        .rev()
        .find(|&&line| line < click_line)
        .copied()?;
    let next = starts.iter().find(|&&line| line > click_line).copied()?;
    let start_line = prev + 1;
    let end_line = next - 1;
    if end_line < start_line {
        return None;
    }
    Some(crate::SelectionRange {
        start: crate::Point::new(start_line, 0),
        end: crate::Point::new(end_line, last_column),
        is_block: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn last_active_source_line(text: &str, needle: &str) -> Option<usize> {
        let hits: Vec<usize> = text
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                let t = line.trim();
                if t.starts_with('#') || !t.contains(needle) {
                    return None;
                }
                let is_source = t.starts_with("source ")
                    || t.starts_with(". ")
                    || t.contains(" source ")
                    || t.contains(" . ");
                is_source.then_some(i)
            })
            .collect();
        hits.last().copied()
    }

    fn assert_emits_osc133(script: &str, shell: &str) {
        for kind in ["A", "B", "C", "D"] {
            assert!(
                script.contains(&format!("133;{kind}")),
                "{shell} inject script must emit OSC 133 {kind}, got:\n{script}"
            );
        }
    }

    #[test]
    fn zsh_inject_script_emits_osc133_abcd() {
        assert_emits_osc133(inject_script(InjectShell::Zsh), "zsh");
    }

    #[test]
    fn bash_inject_script_emits_osc133_abcd() {
        assert_emits_osc133(inject_script(InjectShell::Bash), "bash");
    }

    #[test]
    fn fish_inject_script_emits_osc133_abcd() {
        assert_emits_osc133(inject_script(InjectShell::Fish), "fish");
    }

    /// `$PS1` must appear before `133;B` on the assignment so B is drawn after
    /// the prompt, not at column 0.
    fn ps1_assignment_appends_b(script: &str) -> bool {
        script.lines().any(|line| {
            let trimmed = line.trim();
            if !trimmed.contains("PS1=") || !trimmed.contains("133;B") {
                return false;
            }
            let ps1_ref = trimmed.find("$PS1").or_else(|| trimmed.find("${PS1}"));
            let b = trimmed.find("133;B");
            matches!((ps1_ref, b), (Some(p), Some(q)) if p < q)
        })
    }

    #[test]
    fn zsh_b_is_literal_osc_not_command_substitution() {
        let zsh = inject_script(InjectShell::Zsh);
        let b_line = zsh
            .lines()
            .find(|line| line.contains("PS1=") && line.contains("133;B"))
            .expect("zsh must assign 133;B onto PS1");
        assert!(
            !b_line.contains("$("),
            "stock zsh has prompt_subst off; $(printf ...) is printed literally and never emits OSC 133 B: {b_line}"
        );
        assert!(
            b_line.contains(r"\e]133;B") || b_line.contains(r"\033]133;B"),
            "zsh B must be an ANSI-C / literal OSC, not a command: {b_line}"
        );
    }

    #[test]
    fn inject_scripts_emit_b_after_the_prompt_not_before() {
        assert!(
            ps1_assignment_appends_b(inject_script(InjectShell::Zsh)),
            "zsh must append 133;B to PS1, not prepend it:\n{}",
            inject_script(InjectShell::Zsh)
        );
        assert!(
            ps1_assignment_appends_b(inject_script(InjectShell::Bash)),
            "bash must append 133;B to PS1, not prepend it:\n{}",
            inject_script(InjectShell::Bash)
        );
        let fish = inject_script(InjectShell::Fish);
        assert!(
            fish.contains("functions -c fish_prompt")
                || fish.contains("functions --copy fish_prompt"),
            "fish must wrap fish_prompt so B is printed after the prompt:\n{fish}"
        );
        if let Some(event_fn) = fish.split("function __sleipnir_precmd").nth(1) {
            let body = event_fn.split("end").next().unwrap_or("");
            assert!(
                !body.contains("133;B"),
                "fish_prompt event must not emit B before the prompt is drawn:\n{body}"
            );
        }
    }

    #[test]
    fn from_program_recognizes_supported_shells_only() {
        assert_eq!(
            InjectShell::from_program("/bin/zsh"),
            Some(InjectShell::Zsh)
        );
        assert_eq!(
            InjectShell::from_program("/usr/local/bin/bash"),
            Some(InjectShell::Bash)
        );
        assert_eq!(InjectShell::from_program("fish"), Some(InjectShell::Fish));
        assert_eq!(InjectShell::from_program("python3"), None);
        assert_eq!(InjectShell::from_program("/bin/sh"), None);
    }

    #[test]
    fn wrap_off_leaves_argv_and_does_not_set_inject_env() {
        let mut env = collections::HashMap::default();
        let (program, args) = wrap_shell_for_inject("zsh", None, &mut env, false);
        assert_eq!(program, "zsh");
        assert!(args.is_none());
        assert!(!env.contains_key("SLEIPNIR_SHELL_INTEGRATION"));
        assert!(!env.contains_key("ZDOTDIR"));
    }

    #[test]
    fn wrap_on_unsupported_shell_is_a_noop() {
        let mut env = collections::HashMap::default();
        let (program, args) =
            wrap_shell_for_inject("python3", Some(vec!["-i".into()]), &mut env, true);
        assert_eq!(program, "python3");
        assert_eq!(args, Some(vec!["-i".into()]));
        assert!(!env.contains_key("SLEIPNIR_SHELL_INTEGRATION"));
    }

    #[test]
    fn wrap_on_zsh_writes_script_and_sets_zdotdir() {
        let dir = std::env::temp_dir().join(format!(
            "sleipnir-osc133-{}-{}",
            std::process::id(),
            "zshwrap"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut env = collections::HashMap::default();
        let (program, args) = wrap_shell_for_inject_in("zsh", None, &mut env, true, &dir);
        assert_eq!(program, "zsh");
        assert!(args.is_none());
        assert_eq!(
            env.get("SLEIPNIR_SHELL_INTEGRATION").map(String::as_str),
            Some("1")
        );
        let zdot = env.get("ZDOTDIR").expect("ZDOTDIR");
        assert!(zdot.starts_with(dir.to_str().unwrap()));
        let script = std::fs::read_to_string(dir.join("osc133.zsh")).unwrap();
        assert!(script.contains("133;A"));
        let zshrc = std::fs::read_to_string(Path::new(zdot).join(".zshrc")).unwrap();
        assert!(zshrc.contains("osc133.zsh"));
        let user_line = last_active_source_line(&zshrc, ".zshrc")
            .expect("wrapper must source the user's .zshrc");
        let inject_line = last_active_source_line(&zshrc, "osc133.zsh")
            .expect("wrapper must source inject script");
        assert!(
            user_line < inject_line,
            "user .zshrc must be sourced before inject so PS1 wrap survives:\n{zshrc}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrap_on_bash_uses_rcfile_and_writes_script() {
        let dir = std::env::temp_dir().join(format!(
            "sleipnir-osc133-{}-{}",
            std::process::id(),
            "bashwrap"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut env = collections::HashMap::default();
        let (program, args) = wrap_shell_for_inject_in("bash", None, &mut env, true, &dir);
        assert_eq!(program, "bash");
        let args = args.expect("bash --rcfile");
        assert!(args.windows(2).any(|w| w[0] == "--rcfile"));
        let script = std::fs::read_to_string(dir.join("osc133.bash")).unwrap();
        assert!(script.contains("133;C"));
        let rc = std::fs::read_to_string(dir.join("bash.rc")).unwrap();
        let user_line =
            last_active_source_line(&rc, ".bashrc").expect("wrapper must source ~/.bashrc");
        let inject_line =
            last_active_source_line(&rc, "osc133.bash").expect("wrapper must source inject script");
        assert!(
            user_line < inject_line,
            "user bashrc must be sourced before inject so PS1 wrap survives:\n{rc}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrap_on_fish_uses_init_command() {
        let dir = std::env::temp_dir().join(format!(
            "sleipnir-osc133-{}-{}",
            std::process::id(),
            "fishwrap"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut env = collections::HashMap::default();
        let (program, args) = wrap_shell_for_inject_in("fish", None, &mut env, true, &dir);
        assert_eq!(program, "fish");
        let args = args.expect("fish -C");
        assert!(args.iter().any(|a| a == "-C" || a.starts_with("source ")));
        let script = std::fs::read_to_string(dir.join("osc133.fish")).unwrap();
        assert!(script.contains("133;D"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrap_skips_noninteractive_dash_c() {
        let dir = std::env::temp_dir().join(format!(
            "sleipnir-osc133-{}-{}",
            std::process::id(),
            "dashc"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut env = collections::HashMap::default();
        let args = Some(vec!["-c".into(), "echo hi".into()]);
        let (program, out) = wrap_shell_for_inject_in("zsh", args.clone(), &mut env, true, &dir);
        assert_eq!(program, "zsh");
        assert_eq!(out, args);
        assert!(!env.contains_key("SLEIPNIR_SHELL_INTEGRATION"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn move_req(click_col: usize, cursor_col: usize) -> ClickToMove {
        ClickToMove {
            click_line: 5,
            click_column: click_col,
            cursor_line: 5,
            cursor_column: cursor_col,
            prompt_line: Some(5),
            prompt_prefix_cols: 8,
            alt_screen: false,
        }
    }

    #[test]
    fn click_to_move_sends_right_from_cursor_to_cell() {
        let bytes = click_to_move_sequence(move_req(20, 10)).expect("inside prompt");
        // Line editors treat one CSI C as one cell; CSI n C is not N arrows.
        assert_eq!(bytes, b"\x1b[C".repeat(10));
    }

    #[test]
    fn click_to_move_sends_left_from_cursor_to_cell() {
        let bytes = click_to_move_sequence(move_req(10, 16)).expect("inside prompt");
        assert_eq!(bytes, b"\x1b[D".repeat(6));
    }

    #[test]
    fn click_to_move_does_not_emit_counted_csi() {
        let bytes = click_to_move_sequence(move_req(20, 10)).expect("inside prompt");
        assert!(
            !bytes.windows(3).any(|w| w == b"\x1b[1") && !bytes.contains(&b'0'),
            "must not emit CSI n C/D (got {bytes:?})"
        );
    }

    #[test]
    fn click_to_move_same_cell_is_empty_sequence() {
        let bytes = click_to_move_sequence(move_req(12, 12)).expect("inside prompt");
        assert!(bytes.is_empty());
    }

    #[test]
    fn click_to_move_noop_on_alt_screen() {
        let mut req = move_req(20, 10);
        req.alt_screen = true;
        assert_eq!(click_to_move_sequence(req), None);
    }

    #[test]
    fn click_to_move_noop_without_prompt_markers() {
        let mut req = move_req(20, 10);
        req.prompt_line = None;
        assert_eq!(click_to_move_sequence(req), None);
    }

    #[test]
    fn click_to_move_noop_outside_current_prompt_line() {
        let mut req = move_req(20, 10);
        req.click_line = 4;
        assert_eq!(click_to_move_sequence(req), None);
        req.click_line = 5;
        req.cursor_line = 6;
        assert_eq!(click_to_move_sequence(req), None);
    }

    #[test]
    fn click_to_move_noop_on_prompt_prefix() {
        let req = move_req(3, 10);
        assert_eq!(click_to_move_sequence(req), None);
    }

    fn marker(kind: crate::Osc133Kind, line: i32) -> crate::Osc133Marker {
        crate::Osc133Marker {
            kind,
            line: Some(line),
            column: Some(0),
        }
    }

    #[test]
    fn absolute_to_grid_line_subtracts_history() {
        // No scrollback: absolute == grid.
        assert_eq!(absolute_to_grid_line(5, 0), 5);
        // 100 lines of scrollback: absolute row 110 is grid row 10.
        assert_eq!(absolute_to_grid_line(110, 100), 10);
        // A prompt scrolled up into history maps to a negative grid line.
        assert_eq!(absolute_to_grid_line(40, 100), -60);
    }

    #[test]
    fn click_to_move_matches_prompt_after_history_is_converted() {
        // The prompt marker was recorded absolute at row 105 under 100 lines of
        // scrollback, i.e. grid row 5. The mouse click and cursor are already
        // grid coordinates at row 5. Converting the prompt line to grid first
        // is what makes the lines line up.
        let history_size = 100;
        let prompt_abs = 105;
        let mut req = move_req(20, 10);
        req.click_line = 5;
        req.cursor_line = 5;
        req.prompt_line = Some(absolute_to_grid_line(prompt_abs, history_size));
        let bytes = click_to_move_sequence(req).expect("grid prompt line must match");
        assert_eq!(bytes, b"\x1b[C".repeat(10));
    }

    #[test]
    fn command_output_range_from_c_and_d() {
        use crate::Osc133Kind::{CommandExecuted, CommandFinished};
        let markers = [
            marker(CommandExecuted, 10),
            marker(CommandFinished { status: Some(0) }, 20),
        ];
        // No scrollback: absolute marker lines equal grid lines.
        let range = command_output_range(&markers, 15, 80, 0).expect("inside output");
        assert_eq!(range.start, crate::Point::new(10, 0));
        assert_eq!(range.end, crate::Point::new(20, 80));
        assert!(!range.is_block);
        assert!(command_output_range(&markers, 25, 80, 0).is_none());
    }

    #[test]
    fn command_output_range_falls_back_to_prompt_pair() {
        use crate::Osc133Kind::PromptStart;
        let markers = [marker(PromptStart, 0), marker(PromptStart, 15)];
        let range = command_output_range(&markers, 8, 40, 0).expect("between prompts");
        assert_eq!(range.start, crate::Point::new(1, 0));
        assert_eq!(range.end, crate::Point::new(14, 40));
    }

    #[test]
    fn command_output_range_matches_grid_click_against_absolute_markers() {
        use crate::Osc133Kind::{CommandExecuted, CommandFinished};
        // Markers are stored absolute (cursor.line + history_size). With 100
        // lines of scrollback the command output lives at absolute rows
        // 110..120, which map to grid rows 10..20.
        let history_size = 100;
        let markers = [
            marker(CommandExecuted, 110),
            marker(CommandFinished { status: Some(0) }, 120),
        ];
        // A grid-coordinate click at row 15 (inside the output) must match, and
        // the returned range must be back in grid coordinates for selection.
        let range =
            command_output_range(&markers, 15, 80, history_size).expect("grid click inside output");
        assert_eq!(range.start, crate::Point::new(10, 0));
        assert_eq!(range.end, crate::Point::new(20, 80));
        // A grid click outside the output must not match.
        assert!(command_output_range(&markers, 25, 80, history_size).is_none());
    }

    #[test]
    fn modifier_triple_click_selects_output_plain_selects_lines() {
        use crate::Osc133Kind::{CommandExecuted, CommandFinished};
        let markers = [
            marker(CommandExecuted, 10),
            marker(CommandFinished { status: Some(0) }, 20),
        ];
        match triple_click_kind(true, &markers, 15, 80, 0) {
            TripleClickKind::CommandOutput(range) => {
                assert_eq!(range.start.line, 10);
                assert_eq!(range.end.line, 20);
            }
            TripleClickKind::Lines => panic!("modifier triple-click should use output range"),
        }
        assert_eq!(
            triple_click_kind(false, &markers, 15, 80, 0),
            TripleClickKind::Lines
        );
        assert_eq!(
            triple_click_kind(true, &[], 15, 80, 0),
            TripleClickKind::Lines
        );
    }
}
