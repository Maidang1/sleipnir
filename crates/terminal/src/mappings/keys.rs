use std::borrow::Cow;

/// The mappings defined in this file where created from reading the alacritty source
use gpui::Keystroke;

use crate::Modes;

#[derive(Debug, PartialEq, Eq)]
enum TerminalModifiers {
    None,
    Alt,
    Ctrl,
    Shift,
    CtrlShift,
    Other,
}

impl TerminalModifiers {
    fn new(ks: &Keystroke) -> Self {
        match (
            ks.modifiers.alt,
            ks.modifiers.control,
            ks.modifiers.shift,
            ks.modifiers.platform,
        ) {
            (false, false, false, false) => TerminalModifiers::None,
            (true, false, false, false) => TerminalModifiers::Alt,
            (false, true, false, false) => TerminalModifiers::Ctrl,
            (false, false, true, false) => TerminalModifiers::Shift,
            (false, true, true, false) => TerminalModifiers::CtrlShift,
            _ => TerminalModifiers::Other,
        }
    }

    fn any(&self) -> bool {
        match &self {
            TerminalModifiers::None => false,
            TerminalModifiers::Alt => true,
            TerminalModifiers::Ctrl => true,
            TerminalModifiers::Shift => true,
            TerminalModifiers::CtrlShift => true,
            TerminalModifiers::Other => true,
        }
    }
}

pub(crate) fn to_esc_str(
    keystroke: &Keystroke,
    mode: Modes,
    option_as_meta: bool,
) -> Option<Cow<'static, str>> {
    let modifiers = TerminalModifiers::new(keystroke);

    const FUNCTION_KEYS: [&str; 20] = [
        "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "f13", "f14",
        "f15", "f16", "f17", "f18", "f19", "f20",
    ];
    const FUNCTION_CODES: [u8; 20] = [
        11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34,
    ];
    if let Some(index) = FUNCTION_KEYS.iter().position(|key| *key == keystroke.key) {
        let code = modifier_code(keystroke);
        let sequence = match (index < 4, modifiers.any()) {
            (true, false) => format!("\x1bO{}", char::from(b'P' + index as u8)),
            (true, true) => format!("\x1b[1;{code}{}", char::from(b'P' + index as u8)),
            (false, false) => format!("\x1b[{}~", FUNCTION_CODES[index]),
            (false, true) => format!("\x1b[{};{code}~", FUNCTION_CODES[index]),
        };
        return Some(sequence.into());
    }
    if matches!(
        modifiers,
        TerminalModifiers::Ctrl | TerminalModifiers::CtrlShift
    ) {
        if let [letter] = keystroke.key.as_bytes() {
            if letter.is_ascii_alphabetic() {
                return Some(
                    char::from(letter.to_ascii_lowercase() - b'a' + 1)
                        .to_string()
                        .into(),
                );
            }
        }
    }

    // Manual Bindings including modifiers
    let manual_esc_str: Option<&'static str> = match (keystroke.key.as_ref(), &modifiers) {
        //Basic special keys
        ("tab", TerminalModifiers::None) => Some("\x09"),
        ("escape", TerminalModifiers::None) => Some("\x1b"),
        ("enter", TerminalModifiers::None) => Some("\x0d"),
        ("enter", TerminalModifiers::Shift) => Some("\x0a"),
        ("enter", TerminalModifiers::Alt) => Some("\x1b\x0d"),
        ("backspace", TerminalModifiers::None) => Some("\x7f"),
        //Interesting escape codes
        ("tab", TerminalModifiers::Shift) => Some("\x1b[Z"),
        ("backspace", TerminalModifiers::Ctrl) => Some("\x08"),
        ("backspace", TerminalModifiers::Alt) => Some("\x1b\x7f"),
        ("backspace", TerminalModifiers::Shift) => Some("\x7f"),
        ("space", TerminalModifiers::Ctrl) => Some("\x00"),
        ("home", TerminalModifiers::None) if mode.contains(Modes::APP_CURSOR) => Some("\x1bOH"),
        ("home", TerminalModifiers::None) if !mode.contains(Modes::APP_CURSOR) => Some("\x1b[H"),
        ("end", TerminalModifiers::None) if mode.contains(Modes::APP_CURSOR) => Some("\x1bOF"),
        ("end", TerminalModifiers::None) if !mode.contains(Modes::APP_CURSOR) => Some("\x1b[F"),
        ("up", TerminalModifiers::None) if mode.contains(Modes::APP_CURSOR) => Some("\x1bOA"),
        ("up", TerminalModifiers::None) if !mode.contains(Modes::APP_CURSOR) => Some("\x1b[A"),
        ("down", TerminalModifiers::None) if mode.contains(Modes::APP_CURSOR) => Some("\x1bOB"),
        ("down", TerminalModifiers::None) if !mode.contains(Modes::APP_CURSOR) => Some("\x1b[B"),
        ("right", TerminalModifiers::None) if mode.contains(Modes::APP_CURSOR) => Some("\x1bOC"),
        ("right", TerminalModifiers::None) if !mode.contains(Modes::APP_CURSOR) => Some("\x1b[C"),
        ("left", TerminalModifiers::None) if mode.contains(Modes::APP_CURSOR) => Some("\x1bOD"),
        ("left", TerminalModifiers::None) if !mode.contains(Modes::APP_CURSOR) => Some("\x1b[D"),
        ("back", TerminalModifiers::None) => Some("\x7f"),
        ("insert", TerminalModifiers::None) => Some("\x1b[2~"),
        ("delete", TerminalModifiers::None) => Some("\x1b[3~"),
        ("pageup", TerminalModifiers::None) => Some("\x1b[5~"),
        ("pagedown", TerminalModifiers::None) => Some("\x1b[6~"),
        // NumpadEnter, Action::Esc("\n".into());
        //Mappings for caret notation keys
        ("@", TerminalModifiers::Ctrl) => Some("\x00"), //0
        ("[", TerminalModifiers::Ctrl) => Some("\x1b"), //27
        ("\\", TerminalModifiers::Ctrl) => Some("\x1c"), //28
        ("]", TerminalModifiers::Ctrl) => Some("\x1d"), //29
        ("^", TerminalModifiers::Ctrl) => Some("\x1e"), //30
        ("_", TerminalModifiers::Ctrl) => Some("\x1f"), //31
        ("?", TerminalModifiers::Ctrl) => Some("\x7f"), //127
        _ => None,
    };
    if let Some(esc_str) = manual_esc_str {
        return Some(Cow::Borrowed(esc_str));
    }

    // Automated bindings applying modifiers
    if modifiers.any() {
        let modifier_code = modifier_code(keystroke);
        let modified_esc_str = match keystroke.key.as_ref() {
            "up" => Some(format!("\x1b[1;{}A", modifier_code)),
            "down" => Some(format!("\x1b[1;{}B", modifier_code)),
            "right" => Some(format!("\x1b[1;{}C", modifier_code)),
            "left" => Some(format!("\x1b[1;{}D", modifier_code)),
            "insert" => Some(format!("\x1b[2;{}~", modifier_code)),
            "pageup" => Some(format!("\x1b[5;{}~", modifier_code)),
            "pagedown" => Some(format!("\x1b[6;{}~", modifier_code)),
            "end" => Some(format!("\x1b[1;{}F", modifier_code)),
            "home" => Some(format!("\x1b[1;{}H", modifier_code)),
            _ => None,
        };
        if let Some(esc_str) = modified_esc_str {
            return Some(Cow::Owned(esc_str));
        }
    }

    if !cfg!(target_os = "macos") || option_as_meta {
        let is_alt_lowercase_ascii =
            modifiers == TerminalModifiers::Alt && keystroke.key.is_ascii();
        let is_alt_uppercase_ascii =
            keystroke.modifiers.alt && keystroke.modifiers.shift && keystroke.key.is_ascii();
        if is_alt_lowercase_ascii || is_alt_uppercase_ascii {
            let key = if is_alt_uppercase_ascii {
                &keystroke.key.to_ascii_uppercase()
            } else {
                &keystroke.key
            };
            return Some(Cow::Owned(format!("\x1b{}", key)));
        }
    }

    None
}

///   Code     Modifiers
/// ---------+---------------------------
///    2     | Shift
///    3     | Alt
///    4     | Shift + Alt
///    5     | Control
///    6     | Shift + Control
///    7     | Alt + Control
///    8     | Shift + Alt + Control
/// ---------+---------------------------
/// from: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-PC-Style-Function-Keys
fn modifier_code(keystroke: &Keystroke) -> u32 {
    let mut modifier_code = 0;
    if keystroke.modifiers.shift {
        modifier_code |= 1;
    }
    if keystroke.modifiers.alt {
        modifier_code |= 1 << 1;
    }
    if keystroke.modifiers.control {
        modifier_code |= 1 << 2;
    }
    modifier_code + 1
}

#[cfg(test)]
mod test {
    use gpui::Modifiers;

    use super::*;

    #[test]
    fn function_keys_use_canonical_spelling_for_every_modifier() {
        let codes = [
            11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34,
        ];
        for mask in 0..8 {
            for (i, code) in codes.iter().enumerate() {
                let key = Keystroke {
                    modifiers: Modifiers {
                        shift: mask & 1 != 0,
                        alt: mask & 2 != 0,
                        control: mask & 4 != 0,
                        ..Default::default()
                    },
                    key: format!("f{}", i + 1),
                    key_char: None,
                };
                let expected = if i < 4 && mask == 0 {
                    format!("\x1bO{}", char::from(b'P' + i as u8))
                } else if i < 4 {
                    format!("\x1b[1;{}{}", mask + 1, char::from(b'P' + i as u8))
                } else if mask == 0 {
                    format!("\x1b[{}~", code)
                } else {
                    format!("\x1b[{};{}~", code, mask + 1)
                };
                assert_eq!(
                    to_esc_str(&key, Modes::NONE, true).as_deref(),
                    Some(expected.as_str()),
                    "{key:?}"
                );
            }
        }
    }

    #[test]
    fn control_letters_accept_upper_and_lower_input_spelling() {
        for byte in b'a'..=b'z' {
            for key in [char::from(byte), char::from(byte).to_ascii_uppercase()] {
                for shift in [false, true] {
                    let key = Keystroke {
                        modifiers: Modifiers {
                            control: true,
                            shift,
                            ..Default::default()
                        },
                        key: key.to_string(),
                        key_char: None,
                    };
                    let expected = char::from(byte - b'a' + 1).to_string();
                    assert_eq!(
                        to_esc_str(&key, Modes::NONE, false).as_deref(),
                        Some(expected.as_str())
                    );
                }
            }
        }
    }

    #[test]
    fn test_plain_inputs() {
        let ks = Keystroke {
            modifiers: Modifiers {
                control: false,
                alt: false,
                shift: false,
                platform: false,
                function: false,
            },
            key: "🖖🏻".to_string(), //2 char string
            key_char: None,
        };
        assert_eq!(to_esc_str(&ks, Modes::NONE, false), None);
    }

    #[test]
    fn test_application_mode() {
        let app_cursor = Modes::APP_CURSOR;
        let none = Modes::NONE;

        let up = Keystroke::parse("up").unwrap();
        let down = Keystroke::parse("down").unwrap();
        let left = Keystroke::parse("left").unwrap();
        let right = Keystroke::parse("right").unwrap();

        assert_eq!(to_esc_str(&up, none, false), Some("\x1b[A".into()));
        assert_eq!(to_esc_str(&down, none, false), Some("\x1b[B".into()));
        assert_eq!(to_esc_str(&right, none, false), Some("\x1b[C".into()));
        assert_eq!(to_esc_str(&left, none, false), Some("\x1b[D".into()));

        assert_eq!(to_esc_str(&up, app_cursor, false), Some("\x1bOA".into()));
        assert_eq!(to_esc_str(&down, app_cursor, false), Some("\x1bOB".into()));
        assert_eq!(to_esc_str(&right, app_cursor, false), Some("\x1bOC".into()));
        assert_eq!(to_esc_str(&left, app_cursor, false), Some("\x1bOD".into()));

        let home = Keystroke::parse("home").unwrap();
        let end = Keystroke::parse("end").unwrap();
        assert_eq!(to_esc_str(&home, none, false), Some("\x1b[H".into()));
        assert_eq!(to_esc_str(&end, none, false), Some("\x1b[F".into()));
        assert_eq!(to_esc_str(&home, app_cursor, false), Some("\x1bOH".into()));
        assert_eq!(to_esc_str(&end, app_cursor, false), Some("\x1bOF".into()));

        let shift_up = Keystroke::parse("shift-up").unwrap();
        let shift_down = Keystroke::parse("shift-down").unwrap();
        let shift_home = Keystroke::parse("shift-home").unwrap();
        let shift_end = Keystroke::parse("shift-end").unwrap();
        assert_eq!(to_esc_str(&shift_up, none, false), Some("\x1b[1;2A".into()));
        assert_eq!(
            to_esc_str(&shift_down, none, false),
            Some("\x1b[1;2B".into())
        );
        assert_eq!(
            to_esc_str(&shift_home, none, false),
            Some("\x1b[1;2H".into())
        );
        assert_eq!(
            to_esc_str(&shift_end, none, false),
            Some("\x1b[1;2F".into())
        );
    }

    #[test]
    fn test_ctrl_codes() {
        let letters_lower = 'a'..='z';
        let letters_upper = 'A'..='Z';
        let mode = Modes::APP_CURSOR;

        for (lower, upper) in letters_lower.zip(letters_upper) {
            assert_eq!(
                to_esc_str(
                    &Keystroke::parse(&format!("ctrl-shift-{}", lower)).unwrap(),
                    mode,
                    false
                ),
                to_esc_str(
                    &Keystroke::parse(&format!("ctrl-{}", upper)).unwrap(),
                    mode,
                    false
                ),
                "On letter: {}/{}",
                lower,
                upper
            )
        }
    }

    #[test]
    fn plain_control_c_and_v_emit_terminal_control_bytes() {
        assert_eq!(
            to_esc_str(&Keystroke::parse("ctrl-c").unwrap(), Modes::NONE, false),
            Some("\x03".into())
        );
        assert_eq!(
            to_esc_str(&Keystroke::parse("ctrl-v").unwrap(), Modes::NONE, false),
            Some("\x16".into())
        );
    }

    #[test]
    fn alt_is_meta() {
        let ascii_printable = ' '..='~';
        for character in ascii_printable {
            assert_eq!(
                to_esc_str(
                    &Keystroke::parse(&format!("alt-{}", character)).unwrap(),
                    Modes::NONE,
                    true
                )
                .unwrap(),
                format!("\x1b{}", character)
            );
        }

        let gpui_keys = [
            "up", "down", "right", "left", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9",
            "f10", "f11", "f12", "f13", "f14", "f15", "f16", "f17", "f18", "f19", "f20", "insert",
            "pageup", "pagedown", "end", "home",
        ];

        for key in gpui_keys {
            assert_ne!(
                to_esc_str(
                    &Keystroke::parse(&format!("alt-{}", key)).unwrap(),
                    Modes::NONE,
                    true
                )
                .unwrap(),
                format!("\x1b{}", key)
            );
        }
    }

    #[test]
    fn test_shift_enter_newline() {
        let shift_enter = Keystroke::parse("shift-enter").unwrap();
        let regular_enter = Keystroke::parse("enter").unwrap();
        let mode = Modes::NONE;

        // Shift-enter should send line feed (newline)
        assert_eq!(to_esc_str(&shift_enter, mode, false), Some("\x0a".into()));

        // Regular enter should still send carriage return
        assert_eq!(to_esc_str(&regular_enter, mode, false), Some("\x0d".into()));
    }

    #[test]
    fn test_modifier_code_calc() {
        //   Code     Modifiers
        // ---------+---------------------------
        //    2     | Shift
        //    3     | Alt
        //    4     | Shift + Alt
        //    5     | Control
        //    6     | Shift + Control
        //    7     | Alt + Control
        //    8     | Shift + Alt + Control
        // ---------+---------------------------
        // from: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-PC-Style-Function-Keys
        assert_eq!(2, modifier_code(&Keystroke::parse("shift-a").unwrap()));
        assert_eq!(3, modifier_code(&Keystroke::parse("alt-a").unwrap()));
        assert_eq!(4, modifier_code(&Keystroke::parse("shift-alt-a").unwrap()));
        assert_eq!(5, modifier_code(&Keystroke::parse("ctrl-a").unwrap()));
        assert_eq!(6, modifier_code(&Keystroke::parse("shift-ctrl-a").unwrap()));
        assert_eq!(7, modifier_code(&Keystroke::parse("alt-ctrl-a").unwrap()));
        assert_eq!(
            8,
            modifier_code(&Keystroke::parse("shift-ctrl-alt-a").unwrap())
        );
    }
}
