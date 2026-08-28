//! Semantic tone → abstract role (ADR-0017 constraint 1).
//!
//! Raw colours would let plugins ignore the user theme; a theme switch
//! (ADR-0002) would then break every such plugin. This crate never names an
//! `Hsla` or a hex value. A mount point maps [`ToneRole`] onto `ChromeTokens`.

use plugin_protocol::v2::Tone;

/// Abstract paint role for a widget fragment. Mount points convert this to
/// `ChromeTokens`; this crate does not know about concrete colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToneRole {
    #[default]
    Foreground,
    Muted,
    Accent,
    Success,
    Warning,
    Danger,
}

impl ToneRole {
    pub fn from_tone(tone: Tone) -> Self {
        match tone {
            Tone::Fg => Self::Foreground,
            Tone::Dim => Self::Muted,
            Tone::Accent => Self::Accent,
            Tone::Ok => Self::Success,
            Tone::Warn => Self::Warning,
            Tone::Err => Self::Danger,
        }
    }
}

impl From<Tone> for ToneRole {
    fn from(tone: Tone) -> Self {
        Self::from_tone(tone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_protocol_tone_has_a_role() {
        assert_eq!(ToneRole::from_tone(Tone::Fg), ToneRole::Foreground);
        assert_eq!(ToneRole::from_tone(Tone::Dim), ToneRole::Muted);
        assert_eq!(ToneRole::from_tone(Tone::Accent), ToneRole::Accent);
        assert_eq!(ToneRole::from_tone(Tone::Ok), ToneRole::Success);
        assert_eq!(ToneRole::from_tone(Tone::Warn), ToneRole::Warning);
        assert_eq!(ToneRole::from_tone(Tone::Err), ToneRole::Danger);
    }
}
