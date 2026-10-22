use hub_core::chrono::NaiveDateTime;
use hub_core::scripts::{self, NewScript, ScriptMeta, slug};

#[derive(Debug, Clone)]
pub enum FormMessage {
    NameChanged(String),
    DescriptionChanged(String),
    WindowsFileChanged(String),
    LinuxFileChanged(String),
    ConfirmToggled(bool),
    TimeoutChanged(String),
    ScheduleChanged(String),
    SchedulePreset(&'static str),
    ActiveToggled(bool),
}

pub const PRESETS: [(&str, &str); 4] = [
    ("A cada hora", "0 0 * * * *"),
    ("Todo dia 03:00", "0 0 3 * * *"),
    ("Dias úteis 09:00", "0 0 9 * * Mon-Fri"),
    ("Domingo 20:00", "0 0 20 * * Sun"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptForm {
    pub editing: Option<String>,
    pub name: String,
    pub description: String,
    pub windows_file: String,
    pub linux_file: String,
    pub confirm: bool,
    pub timeout: String,
    pub schedule: String,
    pub active: bool,
    files_touched: bool,
}

impl ScriptForm {
    pub fn new() -> Self {
        Self {
            editing: None,
            name: String::new(),
            description: String::new(),
            windows_file: String::new(),
            linux_file: String::new(),
            confirm: false,
            timeout: scripts::DEFAULT_TIMEOUT.to_string(),
            schedule: String::new(),
            active: true,
            files_touched: false,
        }
    }

    pub fn edit(id: &str, meta: &ScriptMeta) -> Self {
        Self {
            editing: Some(id.to_string()),
            name: meta.name.clone(),
            description: meta.description.clone(),
            windows_file: meta.file.windows().unwrap_or_default().to_string(),
            linux_file: meta.file.linux().unwrap_or_default().to_string(),
            confirm: meta.confirm,
            timeout: meta.timeout_secs.to_string(),
            schedule: meta.schedule.clone().unwrap_or_default(),
            active: meta.active,
            files_touched: true,
        }
    }

    pub fn is_new(&self) -> bool {
        self.editing.is_none()
    }

    pub fn update(&mut self, message: FormMessage) {
        match message {
            FormMessage::NameChanged(name) => {
                self.name = name;
                if self.is_new() && !self.files_touched {
                    let id = slug(&self.name);
                    if id.is_empty() {
                        self.windows_file.clear();
                        self.linux_file.clear();
                    } else {
                        self.windows_file = format!("{id}.ps1");
                        self.linux_file = format!("{id}.sh");
                    }
                }
            }
            FormMessage::DescriptionChanged(text) => self.description = text,
            FormMessage::WindowsFileChanged(file) => {
                self.windows_file = file;
                self.files_touched = true;
            }
            FormMessage::LinuxFileChanged(file) => {
                self.linux_file = file;
                self.files_touched = true;
            }
            FormMessage::ConfirmToggled(on) => self.confirm = on,
            FormMessage::TimeoutChanged(text) => {
                self.timeout = text.chars().filter(char::is_ascii_digit).take(6).collect();
            }
            FormMessage::ScheduleChanged(text) => self.schedule = text,
            FormMessage::SchedulePreset(cron) => self.schedule = cron.to_string(),
            FormMessage::ActiveToggled(on) => self.active = on,
        }
    }

    pub fn schedule(&self) -> Option<String> {
        Some(self.schedule.trim().to_string()).filter(|s| !s.is_empty())
    }

    pub fn timeout(&self) -> u64 {
        self.timeout
            .parse()
            .unwrap_or(scripts::DEFAULT_TIMEOUT)
            .max(1)
    }

    pub fn preview(&self, now: NaiveDateTime) -> Result<Option<NaiveDateTime>, String> {
        match self.schedule() {
            None => Ok(None),
            Some(schedule) => scripts::parse_schedule(&schedule)
                .map_err(|e| e.to_string())
                .map(|_| scripts::next_run(&schedule, now)),
        }
    }

    pub fn to_new(&self) -> NewScript {
        let windows = Some(self.windows_file.clone());
        let linux = Some(self.linux_file.clone());
        NewScript {
            name: self.name.clone(),
            description: self.description.clone(),
            windows_content: scripts::template(&self.windows_file),
            linux_content: scripts::template(&self.linux_file),
            windows_file: windows,
            linux_file: linux,
            confirm: self.confirm,
            timeout_secs: self.timeout(),
            schedule: self.schedule(),
            active: self.active,
        }
    }

    pub fn apply(&self, meta: &ScriptMeta) -> ScriptMeta {
        ScriptMeta {
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
            file: meta.file.clone(),
            confirm: self.confirm,
            timeout_secs: self.timeout(),
            schedule: self.schedule(),
            active: self.active,
        }
    }
}

impl Default for ScriptForm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_suggests_file_names_until_edited() {
        let mut form = ScriptForm::new();
        form.update(FormMessage::NameChanged("Limpar Temp".into()));
        assert_eq!(form.windows_file, "limpar-temp.ps1");
        assert_eq!(form.linux_file, "limpar-temp.sh");
        form.update(FormMessage::WindowsFileChanged("x.bat".into()));
        form.update(FormMessage::NameChanged("Outro".into()));
        assert_eq!(form.windows_file, "x.bat");
    }

    #[test]
    fn timeout_keeps_digits_only() {
        let mut form = ScriptForm::new();
        form.update(FormMessage::TimeoutChanged("12a3".into()));
        assert_eq!(form.timeout, "123");
        assert_eq!(form.timeout(), 123);
        form.update(FormMessage::TimeoutChanged(String::new()));
        assert_eq!(form.timeout(), scripts::DEFAULT_TIMEOUT);
    }

    #[test]
    fn preview_shows_next_run_or_error() {
        let now = NaiveDateTime::parse_from_str("2026-10-21 10:00", "%Y-%m-%d %H:%M").unwrap();
        let mut form = ScriptForm::new();
        assert_eq!(form.preview(now), Ok(None));
        form.update(FormMessage::SchedulePreset("0 0 3 * * *"));
        assert_eq!(
            form.preview(now).unwrap().unwrap().to_string(),
            "2026-10-22 03:00:00"
        );
        form.update(FormMessage::ScheduleChanged("toda hora".into()));
        assert!(form.preview(now).is_err());
    }
}
