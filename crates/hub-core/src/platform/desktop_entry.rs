use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub visible: bool,
}

pub const ICON_SIZES: &[&str] = &[
    "48x48", "64x64", "96x96", "128x128", "256x256", "32x32", "512x512", "24x24",
];

pub fn parse(content: &str) -> Option<DesktopEntry> {
    let mut in_main_group = false;
    let mut name = None;
    let mut exec = None;
    let mut icon = None;
    let mut is_application = false;
    let mut visible = true;
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_main_group || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "Icon" if !value.is_empty() => icon = Some(value.to_string()),
            "Type" => is_application = value == "Application",
            "NoDisplay" | "Hidden" if value == "true" => visible = false,
            _ => {}
        }
    }
    if !is_application {
        return None;
    }
    Some(DesktopEntry {
        name: name?,
        exec: exec?,
        icon,
        visible,
    })
}

pub fn icon_candidates(
    icon: &str,
    icon_roots: &[PathBuf],
    pixmap_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let icon = icon.trim();
    let path = Path::new(icon);
    if path.is_absolute() {
        return if is_png(path) {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    let name = icon.strip_suffix(".png").unwrap_or(icon);
    if name.is_empty() || name.contains('/') {
        return Vec::new();
    }
    let file = format!("{name}.png");
    let mut candidates = Vec::new();
    for size in ICON_SIZES {
        for root in icon_roots {
            candidates.push(root.join("hicolor").join(size).join("apps").join(&file));
        }
    }
    for root in pixmap_roots {
        candidates.push(root.join(&file));
    }
    candidates
}

fn is_png(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
}

pub fn exec_to_args(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut in_quotes = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '%' => {
                if chars.next() == Some('%') {
                    current.push('%');
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token || !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => current.push(c),
        }
    }
    if has_token || !current.is_empty() {
        args.push(current);
    }
    args.retain(|arg| !arg.is_empty());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_main_group_only() {
        let content = "[Desktop Entry]\nType=Application\nName=Firefox\nName[pt_BR]=Navegador\nExec=firefox %u\n\n[Desktop Action new]\nName=Nova janela\nExec=firefox --new-window\n";
        let entry = parse(content).unwrap();
        assert_eq!(entry.name, "Firefox");
        assert_eq!(entry.exec, "firefox %u");
        assert!(entry.visible);
    }

    #[test]
    fn icon_is_parsed() {
        let entry =
            parse("[Desktop Entry]\nType=Application\nName=X\nExec=x\nIcon=firefox\n").unwrap();
        assert_eq!(entry.icon.as_deref(), Some("firefox"));
        let entry = parse("[Desktop Entry]\nType=Application\nName=X\nExec=x\nIcon=\n").unwrap();
        assert_eq!(entry.icon, None);
    }

    #[test]
    fn icon_candidates_follow_size_preference_then_pixmaps() {
        let roots = vec![
            PathBuf::from("/home/eu/.local/share/icons"),
            PathBuf::from("/usr/share/icons"),
        ];
        let pixmaps = vec![PathBuf::from("/usr/share/pixmaps")];
        let candidates = icon_candidates("firefox", &roots, &pixmaps);
        assert_eq!(candidates.len(), ICON_SIZES.len() * 2 + 1);
        assert_eq!(
            candidates[0],
            PathBuf::from("/home/eu/.local/share/icons/hicolor/48x48/apps/firefox.png")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from("/usr/share/icons/hicolor/48x48/apps/firefox.png")
        );
        assert_eq!(
            candidates.last().unwrap(),
            &PathBuf::from("/usr/share/pixmaps/firefox.png")
        );
        assert_eq!(
            icon_candidates("app.png", &roots, &pixmaps)[0]
                .file_name()
                .unwrap(),
            "app.png"
        );
        assert!(icon_candidates("a/b", &roots, &pixmaps).is_empty());
    }

    #[test]
    fn absolute_icons_are_used_only_when_png() {
        let absolute = std::env::temp_dir().join("icone.png");
        assert_eq!(
            icon_candidates(absolute.to_str().unwrap(), &[], &[]),
            vec![absolute]
        );
        let svg = std::env::temp_dir().join("icone.svg");
        assert!(icon_candidates(svg.to_str().unwrap(), &[], &[]).is_empty());
    }

    #[test]
    fn hidden_entries_are_marked() {
        let entry =
            parse("[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n").unwrap();
        assert!(!entry.visible);
    }

    #[test]
    fn non_applications_are_ignored() {
        assert!(parse("[Desktop Entry]\nType=Link\nName=X\nURL=http://a\n").is_none());
        assert!(parse("[Desktop Entry]\nType=Application\nName=X\n").is_none());
    }

    #[test]
    fn exec_field_codes_are_removed() {
        assert_eq!(
            exec_to_args("code --new-window %F"),
            vec!["code", "--new-window"]
        );
        assert_eq!(exec_to_args("app 100%%"), vec!["app", "100%"]);
    }

    #[test]
    fn exec_quotes_are_respected() {
        assert_eq!(
            exec_to_args("\"/opt/My App/run\" --flag \"a \\\"b\\\"\""),
            vec!["/opt/My App/run", "--flag", "a \"b\""]
        );
    }
}
