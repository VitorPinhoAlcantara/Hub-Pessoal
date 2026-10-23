#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Dev,
    Release,
}

impl Channel {
    pub const fn current() -> Self {
        if cfg!(feature = "distribuicao") {
            Self::Release
        } else {
            Self::Dev
        }
    }

    pub const fn app_name(self) -> &'static str {
        match self {
            Self::Dev => "Hub Pessoal Dev",
            Self::Release => "Hub Pessoal",
        }
    }

    pub const fn dir_name(self) -> &'static str {
        match self {
            Self::Dev => "hub-dev",
            Self::Release => "hub",
        }
    }

    pub const fn default_mcp_port(self) -> u16 {
        match self {
            Self::Dev => 47822,
            Self::Release => 47821,
        }
    }
}

pub const fn app_name() -> &'static str {
    Channel::current().app_name()
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_never_share_folder_or_port() {
        assert_ne!(Channel::Dev.dir_name(), Channel::Release.dir_name());
        assert_ne!(
            Channel::Dev.default_mcp_port(),
            Channel::Release.default_mcp_port()
        );
        assert!(Channel::Dev.app_name().ends_with("Dev"));
    }
}
