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
    Calendar => "calendar",
    ChevronDown => "chevron-down",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    Command => "command",
    File => "file",
    FilePen => "file-pen",
    Forward => "forward",
    Inbox => "inbox",
    Keyboard => "keyboard",
    Mail => "mail",
    MailOpen => "mail-open",
    MailUnread => "mail-unread",
    OctagonAlert => "octagon-alert",
    PanelLeft => "panel-left",
    Paperclip => "paperclip",
    Reply => "reply",
    ReplyAll => "reply-all",
    Search => "search",
    Send => "send",
    SquarePen => "square-pen",
    Star => "star",
    StarFilled => "star-filled",
    Trash => "trash",
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
