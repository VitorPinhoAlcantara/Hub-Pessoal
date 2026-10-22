use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use super::{CommandSpec, Os, hide_console};

pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
pub const MAX_TIMEOUT_SECS: u64 = 600;
pub const OUTPUT_LIMIT: usize = 20_000;

const UTF8_PREFIX: &str = "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), program.to_string()]
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

pub fn terminal_command(os: Os, command: &str, has_pwsh: bool) -> CommandSpec {
    match os {
        Os::Windows => {
            let program = if has_pwsh { "pwsh" } else { "powershell" };
            CommandSpec::new(program)
                .arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(format!("{UTF8_PREFIX}{command}"))
        }
        Os::Linux => CommandSpec::new("sh").arg("-c").arg(command),
    }
}

pub fn shell_name(os: Os, has_pwsh: bool) -> &'static str {
    match (os, has_pwsh) {
        (Os::Windows, true) => "PowerShell 7 (pwsh)",
        (Os::Windows, false) => "Windows PowerShell",
        (Os::Linux, _) => "sh",
    }
}

pub fn has_pwsh() -> bool {
    cfg!(windows) && find_in_path("pwsh").is_some()
}

pub fn clamp_timeout(seconds: Option<u64>) -> Duration {
    Duration::from_secs(
        seconds
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS),
    )
}

pub fn truncate(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    (text[start..].to_string(), true)
}

pub async fn run_terminal(
    spec: &CommandSpec,
    cwd: &Path,
    timeout: Duration,
) -> std::io::Result<TerminalOutput> {
    let mut command = tokio::process::Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_console(&mut command);
    let child = command.spawn()?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(output) => {
            let output = output?;
            let (stdout, stdout_truncated) =
                truncate(&String::from_utf8_lossy(&output.stdout), OUTPUT_LIMIT);
            let (stderr, stderr_truncated) =
                truncate(&String::from_utf8_lossy(&output.stderr), OUTPUT_LIMIT);
            Ok(TerminalOutput {
                exit_code: output.status.code(),
                stdout,
                stderr,
                timed_out: false,
                stdout_truncated,
                stderr_truncated,
            })
        }
        Err(_) => Ok(TerminalOutput {
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            stdout_truncated: false,
            stderr_truncated: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_uses_powershell_without_profile() {
        let spec = terminal_command(Os::Windows, "Get-Date", true);
        assert_eq!(spec.program, "pwsh");
        assert_eq!(spec.args[0], "-NoProfile");
        assert_eq!(spec.args[1], "-NonInteractive");
        assert_eq!(spec.args[2], "-Command");
        assert!(spec.args[3].to_string_lossy().ends_with("Get-Date"));
        assert_eq!(
            terminal_command(Os::Windows, "x", false).program,
            "powershell"
        );
    }

    #[test]
    fn linux_uses_sh() {
        let spec = terminal_command(Os::Linux, "ls -la", false);
        assert_eq!(spec.program, "sh");
        assert_eq!(spec.args, ["-c", "ls -la"]);
    }

    #[test]
    fn timeout_is_clamped() {
        assert_eq!(clamp_timeout(None), Duration::from_secs(60));
        assert_eq!(clamp_timeout(Some(0)), Duration::from_secs(1));
        assert_eq!(clamp_timeout(Some(9999)), Duration::from_secs(600));
    }

    #[test]
    fn truncate_keeps_the_end_on_char_boundary() {
        assert_eq!(truncate("abc", 5), ("abc".to_string(), false));
        let (text, cut) = truncate("ááá", 3);
        assert!(cut);
        assert_eq!(text, "á");
    }

    #[tokio::test]
    async fn runs_a_real_command_and_reports_exit_code() {
        let os = Os::current();
        let pwsh = has_pwsh();
        let cwd = std::env::temp_dir();
        let ok = run_terminal(
            &terminal_command(os, "echo ola; exit 3", pwsh),
            &cwd,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
        assert!(ok.stdout.contains("ola"), "{ok:?}");
        assert_eq!(ok.exit_code, Some(3));
        assert!(!ok.timed_out);
    }

    #[tokio::test]
    async fn slow_command_times_out() {
        let os = Os::current();
        let command = match os {
            Os::Windows => "Start-Sleep -Seconds 20",
            Os::Linux => "sleep 20",
        };
        let result = run_terminal(
            &terminal_command(os, command, has_pwsh()),
            &std::env::temp_dir(),
            Duration::from_millis(300),
        )
        .await
        .unwrap();
        assert!(result.timed_out);
        assert_eq!(result.exit_code, None);
    }
}
