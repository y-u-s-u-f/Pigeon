//! Icons are compiled into the binary, so there is nothing to find on disk.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon { $($variant),* }

        impl Icon {
            /// The asset path for this glyph drawn with `stroke`.
            pub fn path(self, stroke: Stroke) -> &'static str {
                match (self, stroke) {
                    $(
                        (Icon::$variant, Stroke::Regular) => concat!("icons/", $file, ".svg"),
                        (Icon::$variant, Stroke::Heavy) => concat!("icons/heavy/", $file, ".svg"),
                        (Icon::$variant, Stroke::Light) => concat!("icons/light/", $file, ".svg"),
                    )*
                }
            }
        }

        const FILES: &[(&str, &[u8])] = &[
            $((concat!("icons/", $file, ".svg"), include_bytes!(concat!("../assets/icons/", $file, ".svg")))),*
        ];
    };
}

icons! {
    AlarmClock => "alarm-clock",
    Archive => "archive",
    Bell => "bell",
    Bold => "bold",
    Briefcase => "briefcase",
    Calendar => "calendar",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    Clock => "clock",
    Code => "code",
    Copy => "copy",
    Command => "command",
    Download => "download",
    Ellipsis => "ellipsis",
    File => "file",
    FileArchive => "file-archive",
    FilePen => "file-pen",
    FileSpreadsheet => "file-spreadsheet",
    FileText => "file-text",
    FolderInput => "folder-input",
    Forward => "forward",
    Help => "help",
    Image => "image",
    Important => "important",
    ImportantOutline => "important-outline",
    Inbox => "inbox",
    Italic => "italic",
    Keyboard => "keyboard",
    Layers => "layers",
    Link => "link",
    LogIn => "log-in",
    LogOut => "log-out",
    List => "list",
    ListOrdered => "list-ordered",
    Mail => "mail",
    MailOpen => "mail-open",
    MailUnread => "mail-unread",
    MapPin => "map-pin",
    Megaphone => "megaphone",
    MessagesSquare => "messages-square",
    Minus => "minus",
    OctagonAlert => "octagon-alert",
    PanelLeft => "panel-left",
    PanelTop => "panel-top",
    Phone => "phone",
    Paperclip => "paperclip",
    Plus => "plus",
    RemoveFormatting => "remove-formatting",
    Repeat => "repeat",
    RefreshCw => "refresh-cw",
    Reply => "reply",
    ReplyAll => "reply-all",
    Rows => "rows",
    Search => "search",
    Send => "send",
    Settings => "settings",
    ShieldAlert => "shield-alert",
    SquarePen => "square-pen",
    Star => "star",
    StarFilled => "star-filled",
    Strikethrough => "strikethrough",
    SunMoon => "sun-moon",
    Tag => "tag",
    Text => "text",
    TextQuote => "text-quote",
    Trash => "trash",
    Underline => "underline",
    Undo => "undo",
    User => "user",
    Users => "users",
    Video => "video",
    X => "x",
}

/// Stroke weight by drawn size. The glyphs are drawn on a 24px grid with a
/// 1.75 stroke, which goes hairline at small sizes and heavy at large ones,
/// so small icons get a heavier stroke and hero icons a lighter one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    Heavy,
    Regular,
    Light,
}

impl Stroke {
    /// The stroke for a glyph drawn `size` pixels square (at text scale 1).
    pub fn for_size(size: f32) -> Self {
        if size <= 13. {
            Stroke::Heavy
        } else if size >= 28. {
            Stroke::Light
        } else {
            Stroke::Regular
        }
    }
}

const STROKE: &str = r#"stroke-width="1.75""#;

/// `bytes` with its stroke width swapped for `stroke`'s.
fn restroke(bytes: &'static [u8], stroke: Stroke) -> Cow<'static, [u8]> {
    let width = match stroke {
        Stroke::Regular => return Cow::Borrowed(bytes),
        Stroke::Heavy => r#"stroke-width="2.25""#,
        Stroke::Light => r#"stroke-width="1.25""#,
    };
    let svg = std::str::from_utf8(bytes).unwrap_or_default();
    Cow::Owned(svg.replace(STROKE, width).into_bytes())
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let (path, stroke) = if let Some(file) = path.strip_prefix("icons/heavy/") {
            (format!("icons/{file}"), Stroke::Heavy)
        } else if let Some(file) = path.strip_prefix("icons/light/") {
            (format!("icons/{file}"), Stroke::Light)
        } else {
            (path.to_string(), Stroke::Regular)
        };
        Ok(FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| restroke(bytes, stroke)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(path: &str) -> String {
        let bytes = Assets.load(path).unwrap().unwrap();
        String::from_utf8(bytes.into_owned()).unwrap()
    }

    #[test]
    fn every_glyph_has_the_stroke_to_rewrite() {
        for (name, bytes) in FILES {
            let svg = std::str::from_utf8(bytes).unwrap();
            assert_eq!(svg.matches(STROKE).count(), 1, "{name}");
        }
    }

    #[test]
    fn small_and_hero_sizes_get_their_own_stroke() {
        assert_eq!(Stroke::for_size(12.), Stroke::Heavy);
        assert_eq!(Stroke::for_size(13.), Stroke::Heavy);
        assert_eq!(Stroke::for_size(16.), Stroke::Regular);
        assert_eq!(Stroke::for_size(28.), Stroke::Light);

        let heavy = load(Icon::Archive.path(Stroke::Heavy));
        assert!(heavy.contains(r#"stroke-width="2.25""#));
        assert!(!heavy.contains(STROKE));
        let light = load(Icon::Mail.path(Stroke::Light));
        assert!(light.contains(r#"stroke-width="1.25""#));
        assert_eq!(
            load(Icon::Archive.path(Stroke::Regular)).as_bytes(),
            FILES
                .iter()
                .find(|(n, _)| *n == "icons/archive.svg")
                .unwrap()
                .1
        );
    }

    #[test]
    fn filled_glyphs_stay_filled_with_a_heavier_stroke() {
        for glyph in [Icon::StarFilled, Icon::Important] {
            let svg = load(glyph.path(Stroke::Heavy));
            assert!(svg.contains(r#"fill="currentColor""#), "{glyph:?}");
            assert!(svg.contains(r#"stroke-width="2.25""#), "{glyph:?}");
            assert!(
                svg.ends_with("</svg>\n") || svg.ends_with("</svg>"),
                "{glyph:?}"
            );
        }
        let outline = load(Icon::ImportantOutline.path(Stroke::Heavy));
        assert!(outline.contains(r#"fill="none""#));
    }
}
