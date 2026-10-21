use std::sync::LazyLock;

use iced::widget::svg::Handle;

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($variant),*
        }

        const ALL: &[(Icon, &[u8])] = &[
            $((Icon::$variant, include_bytes!(concat!("../../../assets/icons/", $file, ".svg")))),*
        ];
    };
}

icons! {
    Agenda => "agenda",
    Scripts => "scripts",
    Shortcuts => "atalhos",
    WhatsApp => "whatsapp",
    Ai => "ia",
    Player => "player",
    Settings => "settings",
    Minimize => "minimize",
    Maximize => "maximize",
    Restore => "restore",
    Close => "close",
    Plus => "plus",
    Check => "check",
    Trash => "trash",
    Undo => "undo",
    Repeat => "repeat",
    Clock => "clock",
    Bell => "bell",
    Sun => "sun",
    List => "list",
    Done => "done",
    Flag => "flag",
    Columns => "columns",
}

static HANDLES: LazyLock<Vec<Handle>> = LazyLock::new(|| {
    ALL.iter()
        .map(|(_, bytes)| Handle::from_memory(*bytes))
        .collect()
});

impl Icon {
    pub fn handle(self) -> Handle {
        let index = ALL
            .iter()
            .position(|(icon, _)| *icon == self)
            .unwrap_or_default();
        HANDLES[index].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_valid_svg() {
        for (icon, bytes) in ALL {
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.starts_with("<svg"), "{icon:?}");
            assert!(text.contains("viewBox=\"0 0 24 24\""), "{icon:?}");
        }
    }
}
