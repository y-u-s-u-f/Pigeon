//! Icons are compiled into the binary, so there is nothing to find on disk.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Icon { $($variant),* }

        impl Icon {
            pub fn path(self) -> &'static str {
                match self { $(Icon::$variant => concat!("icons/", $file, ".svg")),* }
            }
        }

        const FILES: &[(&str, &[u8])] = &[
            $((concat!("icons/", $file, ".svg"), include_bytes!(concat!("../assets/icons/", $file, ".svg")))),*
        ];
    };
}

icons! {
    Archive => "archive",
    Bell => "bell",
    Briefcase => "briefcase",
    Calendar => "calendar",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    Clock => "clock",
    Command => "command",
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
    Inbox => "inbox",
    Keyboard => "keyboard",
    Layers => "layers",
    Mail => "mail",
    MailOpen => "mail-open",
    MailUnread => "mail-unread",
    MapPin => "map-pin",
    Minus => "minus",
    OctagonAlert => "octagon-alert",
    PanelLeft => "panel-left",
    Paperclip => "paperclip",
    Plus => "plus",
    Repeat => "repeat",
    Reply => "reply",
    ReplyAll => "reply-all",
    Rows => "rows",
    Search => "search",
    Send => "send",
    SquarePen => "square-pen",
    Star => "star",
    StarFilled => "star-filled",
    Tag => "tag",
    Text => "text",
    Trash => "trash",
    Undo => "undo",
    Users => "users",
    Video => "video",
    X => "x",
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
