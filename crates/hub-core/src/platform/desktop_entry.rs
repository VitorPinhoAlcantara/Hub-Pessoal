#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub visible: bool,
}

pub fn parse(content: &str) -> Option<DesktopEntry> {
    let mut in_main_group = false;
    let mut name = None;
    let mut exec = None;
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
        visible,
    })
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
