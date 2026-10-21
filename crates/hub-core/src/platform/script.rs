use std::path::Path;

use super::{CommandSpec, Os, PlatformError};

pub fn script_command_for(script: &Path, os: Os) -> Result<CommandSpec, PlatformError> {
    let extension = script
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let path = script.as_os_str().to_os_string();
    let spec = match (extension.as_str(), os) {
        ("ps1", Os::Windows) => CommandSpec::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(path),
        ("ps1", Os::Linux) => CommandSpec::new("pwsh")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-File")
            .arg(path),
        ("bat" | "cmd", Os::Windows) => CommandSpec::new("cmd").arg("/C").arg(path),
        ("sh", Os::Windows) => CommandSpec::new("bash").arg(path),
        ("sh", Os::Linux) => CommandSpec::new("sh").arg(path),
        ("py", Os::Windows) => CommandSpec::new("python").arg(path),
        ("py", Os::Linux) => CommandSpec::new("python3").arg(path),
        _ => return Err(PlatformError::NoInterpreter(script.to_path_buf())),
    };
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(script: &str, os: Os) -> String {
        script_command_for(Path::new(script), os)
            .unwrap()
            .program
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn picks_interpreter_by_extension_and_os() {
        assert_eq!(program("a.ps1", Os::Windows), "powershell");
        assert_eq!(program("a.ps1", Os::Linux), "pwsh");
        assert_eq!(program("a.BAT", Os::Windows), "cmd");
        assert_eq!(program("a.sh", Os::Linux), "sh");
        assert_eq!(program("a.sh", Os::Windows), "bash");
        assert_eq!(program("a.py", Os::Linux), "python3");
        assert_eq!(program("a.py", Os::Windows), "python");
    }

    #[test]
    fn script_path_is_last_argument() {
        let spec = script_command_for(Path::new("x/limpar.ps1"), Os::Windows).unwrap();
        assert_eq!(
            spec.args.last().unwrap(),
            Path::new("x/limpar.ps1").as_os_str()
        );
    }

    #[test]
    fn unknown_or_unsupported_extension_is_rejected() {
        assert!(matches!(
            script_command_for(Path::new("a.bat"), Os::Linux),
            Err(PlatformError::NoInterpreter(_))
        ));
        assert!(matches!(
            script_command_for(Path::new("semextensao"), Os::Windows),
            Err(PlatformError::NoInterpreter(_))
        ));
    }
}
