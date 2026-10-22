use serde::{Deserialize, Serialize};

use crate::platform::Os;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PerOs {
    Same(String),
    Split {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        windows: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        linux: Option<String>,
    },
}

impl PerOs {
    pub fn split(windows: Option<String>, linux: Option<String>) -> Self {
        let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        PerOs::Split {
            windows: clean(windows),
            linux: clean(linux),
        }
    }

    pub fn for_os(&self, os: Os) -> Option<&str> {
        match self {
            PerOs::Same(value) => Some(value.as_str()).filter(|v| !v.is_empty()),
            PerOs::Split { windows, linux } => match os {
                Os::Windows => windows.as_deref(),
                Os::Linux => linux.as_deref(),
            },
        }
    }

    pub fn windows(&self) -> Option<&str> {
        self.for_os(Os::Windows)
    }

    pub fn linux(&self) -> Option<&str> {
        self.for_os(Os::Linux)
    }
}

impl Default for PerOs {
    fn default() -> Self {
        PerOs::Split {
            windows: None,
            linux: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Holder {
        alvo: PerOs,
    }

    #[test]
    fn accepts_plain_string_or_table() {
        let same: Holder = toml::from_str("alvo = \"code\"").unwrap();
        assert_eq!(same.alvo.for_os(Os::Windows), Some("code"));
        assert_eq!(same.alvo.for_os(Os::Linux), Some("code"));
        let split: Holder = toml::from_str("alvo = { windows = \"a.ps1\" }").unwrap();
        assert_eq!(split.alvo.windows(), Some("a.ps1"));
        assert_eq!(split.alvo.linux(), None);
    }

    #[test]
    fn split_drops_blank_values() {
        let value = PerOs::split(Some("  ".into()), Some(" x.sh ".into()));
        assert_eq!(value.windows(), None);
        assert_eq!(value.linux(), Some("x.sh"));
        let text = toml::to_string(&Holder { alvo: value }).unwrap();
        assert!(!text.contains("windows"));
    }
}
