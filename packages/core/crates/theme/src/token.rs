//! The tokens: every colour a surface draws, by name. A theme sets any of
//! them in `[theme.ui]` by its key; the rest are derived (see
//! [`crate::Theme::from_section`]).

macro_rules! tokens {
    ($($(#[$doc:meta])* $variant:ident = $key:literal,)*) => {
        /// One colour every surface draws by name rather than by value.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum Token {
            $($(#[$doc])* $variant,)*
        }

        impl Token {
            /// Every token, in declaration order: `ALL[t as usize] == t`.
            pub const ALL: &'static [Token] = &[$(Token::$variant,)*];
            pub const COUNT: usize = Token::ALL.len();

            /// The key a manifest's `[theme.ui]` sets it by.
            pub const fn key(self) -> &'static str {
                match self {
                    $(Token::$variant => $key,)*
                }
            }

            pub fn from_key(key: &str) -> Option<Token> {
                match key {
                    $($key => Some(Token::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

tokens! {
    /// The window behind everything.
    SurfaceBase = "surface-base",
    /// The sidebar and side panels.
    SurfacePanel = "surface-panel",
    /// Cards, inputs and wells.
    SurfaceSunken = "surface-sunken",
    /// Menus, popovers and dialogs.
    SurfaceOverlay = "surface-overlay",
    /// Text and icons at full strength.
    Ink = "ink",
    /// The brightest text: headings and text on a filled control.
    InkBright = "ink-bright",
    InkSecondary = "ink-secondary",
    /// Placeholders, hints and disabled text; never content.
    InkFaint = "ink-faint",
    InkGhost = "ink-ghost",
    /// Hairlines, faintest first.
    LineFaint = "line-faint",
    LineSoft = "line-soft",
    LineBase = "line-base",
    LineStrong = "line-strong",
    LineHover = "line-hover",
    /// Vorn's one accent; at most 5% of a surface.
    Accent = "accent",
    AccentStrong = "accent-strong",
    Danger = "danger",
    StatusSlate = "status-slate",
    StatusBlue = "status-blue",
    StatusSage = "status-sage",
    DiffAdd = "diff-add",
    DiffRemove = "diff-remove",
    /// Behind selected text in the interface.
    Selection = "selection",
    FocusRing = "focus-ring",
    ScrollThumb = "scroll-thumb",
    ScrollThumbHover = "scroll-thumb-hover",
    /// What shadows and scrims are made of.
    Shadow = "shadow",
    Gray100 = "gray-100",
    Gray200 = "gray-200",
    Gray300 = "gray-300",
    Gray400 = "gray-400",
    Gray500 = "gray-500",
    Gray600 = "gray-600",
    Gray700 = "gray-700",
    Gray800 = "gray-800",
    Gray900 = "gray-900",
    Red200 = "red-200",
    Red300 = "red-300",
    Red400 = "red-400",
    Red500 = "red-500",
    Red600 = "red-600",
    Orange400 = "orange-400",
    Orange500 = "orange-500",
    Amber300 = "amber-300",
    Amber400 = "amber-400",
    Amber500 = "amber-500",
    Yellow400 = "yellow-400",
    Yellow500 = "yellow-500",
    Green300 = "green-300",
    Green400 = "green-400",
    Green500 = "green-500",
    Green600 = "green-600",
    Emerald400 = "emerald-400",
    Cyan400 = "cyan-400",
    Sky400 = "sky-400",
    Blue300 = "blue-300",
    Blue400 = "blue-400",
    Blue500 = "blue-500",
    Blue600 = "blue-600",
    Violet400 = "violet-400",
    Violet500 = "violet-500",
    Purple400 = "purple-400",
    Pink400 = "pink-400",
    Pink500 = "pink-500",
    /// Code in the editor, by what each word is.
    SyntaxText = "syntax-text",
    SyntaxComment = "syntax-comment",
    SyntaxString = "syntax-string",
    SyntaxNumber = "syntax-number",
    SyntaxKeyword = "syntax-keyword",
    SyntaxStorage = "syntax-storage",
    SyntaxConstant = "syntax-constant",
    SyntaxFunction = "syntax-function",
    SyntaxVariable = "syntax-variable",
    SyntaxProperty = "syntax-property",
    SyntaxType = "syntax-type",
    SyntaxPunct = "syntax-punct",
    SyntaxList = "syntax-list",
}

/// The families "interface style: neutral" takes from Vorn's own theme
/// instead of the chosen one: the chrome, never the colours that mean
/// something.
impl Token {
    pub const fn is_chrome(self) -> bool {
        use Token::*;
        matches!(
            self,
            SurfaceBase
                | SurfacePanel
                | SurfaceSunken
                | SurfaceOverlay
                | Ink
                | InkBright
                | InkSecondary
                | InkFaint
                | InkGhost
                | LineFaint
                | LineSoft
                | LineBase
                | LineStrong
                | LineHover
                | Selection
                | FocusRing
                | ScrollThumb
                | ScrollThumbHover
                | Shadow
                | Gray100
                | Gray200
                | Gray300
                | Gray400
                | Gray500
                | Gray600
                | Gray700
                | Gray800
                | Gray900
        )
    }

    /// Whether people read text drawn in it, so contrast correction lifts it
    /// to 4.5:1 on the window. Placeholder and hairline tokens stay faint on
    /// purpose; the dark grays are fills.
    pub const fn is_text(self) -> bool {
        use Token::*;
        matches!(
            self,
            Ink | InkBright
                | InkSecondary
                | Accent
                | Danger
                | StatusSlate
                | StatusBlue
                | StatusSage
                | DiffAdd
                | DiffRemove
                | Gray100
                | Gray200
                | Gray300
                | Gray400
                | Red200
                | Red300
                | Red400
                | Orange400
                | Amber300
                | Amber400
                | Yellow400
                | Green300
                | Green400
                | Emerald400
                | Cyan400
                | Sky400
                | Blue300
                | Blue400
                | Violet400
                | Purple400
                | Pink400
                | SyntaxText
                | SyntaxComment
                | SyntaxString
                | SyntaxNumber
                | SyntaxKeyword
                | SyntaxStorage
                | SyntaxConstant
                | SyntaxFunction
                | SyntaxVariable
                | SyntaxProperty
                | SyntaxType
                | SyntaxPunct
                | SyntaxList
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_index() {
        for (i, t) in Token::ALL.iter().enumerate() {
            assert_eq!(*t as usize, i);
            assert_eq!(Token::from_key(t.key()), Some(*t));
        }
        assert_eq!(Token::from_key("surface"), None);
    }
}
