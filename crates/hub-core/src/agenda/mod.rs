pub mod quick_add;
pub mod recurrence;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub use recurrence::Recurrence;

#[derive(Debug, thiserror::Error)]
pub enum AgendaError {
    #[error("erro no banco de dados: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Recurrence(#[from] recurrence::InvalidRecurrence),
    #[error("item {0} não encontrado")]
    NotFound(i64),
    #[error("o título não pode ficar vazio")]
    EmptyTitle,
}

pub type Result<T> = std::result::Result<T, AgendaError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "baixa",
            Priority::Normal => "normal",
            Priority::High => "alta",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "baixa" => Some(Priority::Low),
            "normal" => Some(Priority::Normal),
            "alta" => Some(Priority::High),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub priority: Priority,
    pub completed_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub routine_id: Option<i64>,
    pub missed_at: Option<NaiveDateTime>,
}

impl Task {
    pub fn is_completed(&self) -> bool {
        self.completed_at.is_some()
    }

    pub fn due_at(&self) -> Option<NaiveDateTime> {
        Some(self.date?.and_time(self.time?))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTask {
    pub title: String,
    pub notes: String,
    pub date: Option<NaiveDate>,
    pub time: Option<NaiveTime>,
    pub priority: Priority,
}

impl From<quick_add::ParsedTask> for NewTask {
    fn from(parsed: quick_add::ParsedTask) -> Self {
        Self {
            title: parsed.title,
            notes: String::new(),
            date: parsed.date,
            time: parsed.time,
            priority: parsed.priority,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub recurrence: Recurrence,
    pub time: Option<NaiveTime>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRoutine {
    pub title: String,
    pub notes: String,
    pub recurrence: Recurrence,
    pub time: Option<NaiveTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: i64,
    pub task_id: Option<i64>,
    pub title: String,
    pub notes: String,
    pub fires_at: NaiveDateTime,
    pub fired_at: Option<NaiveDateTime>,
    pub snoozed_until: Option<NaiveDateTime>,
}

impl Reminder {
    pub fn effective_time(&self) -> NaiveDateTime {
        self.snoozed_until.unwrap_or(self.fires_at)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodayView {
    pub overdue: Vec<Task>,
    pub today: Vec<Task>,
    pub undated: Vec<Task>,
}

const TASK_COLUMNS: &str =
    "id, titulo, notas, data, hora, prioridade, concluida_em, criada_em, rotina_id, perdida_em";

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
    let priority: String = row.get(5)?;
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        notes: row.get(2)?,
        date: row.get(3)?,
        time: row.get(4)?,
        priority: Priority::parse(&priority).unwrap_or_default(),
        completed_at: row.get(6)?,
        created_at: row.get(7)?,
        routine_id: row.get(8)?,
        missed_at: row.get(9)?,
    })
}

fn query_tasks(
    conn: &Connection,
    filter: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Task>> {
    let sql = format!("SELECT {TASK_COLUMNS} FROM tarefas WHERE {filter}");
    let mut stmt = conn.prepare(&sql)?;
    let tasks = stmt
        .query_map(params, task_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(tasks)
}

pub fn get_task(conn: &Connection, id: i64) -> Result<Task> {
    query_tasks(conn, "id = ?1", [id])?
        .into_iter()
        .next()
        .ok_or(AgendaError::NotFound(id))
}

pub fn create_task(conn: &Connection, task: NewTask, now: NaiveDateTime) -> Result<Task> {
    create_task_inner(conn, task, None, now)
}

fn create_task_inner(
    conn: &Connection,
    task: NewTask,
    routine_id: Option<i64>,
    now: NaiveDateTime,
) -> Result<Task> {
    let title = task.title.trim();
    if title.is_empty() {
        return Err(AgendaError::EmptyTitle);
    }
    conn.execute(
        "INSERT INTO tarefas (titulo, notas, data, hora, prioridade, criada_em, rotina_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            title,
            task.notes,
            task.date,
            task.time,
            task.priority.as_str(),
            now,
            routine_id
        ],
    )?;
    let created = get_task(conn, conn.last_insert_rowid())?;
    if let Some(at) = created.due_at() {
        insert_reminder(conn, Some(created.id), None, at)?;
    }
    Ok(created)
}

pub fn update_task(conn: &Connection, id: i64, task: NewTask) -> Result<Task> {
    let title = task.title.trim();
    if title.is_empty() {
        return Err(AgendaError::EmptyTitle);
    }
    let changed = conn.execute(
        "UPDATE tarefas SET titulo = ?2, notas = ?3, data = ?4, hora = ?5, prioridade = ?6
         WHERE id = ?1",
        params![
            id,
            title,
            task.notes,
            task.date,
            task.time,
            task.priority.as_str()
        ],
    )?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    let updated = get_task(conn, id)?;
    conn.execute(
        "DELETE FROM lembretes WHERE tarefa_id = ?1 AND disparado_em IS NULL",
        [id],
    )?;
    if let Some(at) = updated.due_at() {
        insert_reminder(conn, Some(id), None, at)?;
    }
    Ok(updated)
}

pub fn complete_task(conn: &Connection, id: i64, now: NaiveDateTime) -> Result<()> {
    let changed = conn.execute(
        "UPDATE tarefas SET concluida_em = ?2 WHERE id = ?1 AND concluida_em IS NULL",
        params![id, now],
    )?;
    if changed == 0 {
        get_task(conn, id)?;
    }
    conn.execute(
        "UPDATE lembretes SET disparado_em = ?2 WHERE tarefa_id = ?1 AND disparado_em IS NULL",
        params![id, now],
    )?;
    Ok(())
}

pub fn uncomplete_task(conn: &Connection, id: i64) -> Result<()> {
    let changed = conn.execute(
        "UPDATE tarefas SET concluida_em = NULL, perdida_em = NULL WHERE id = ?1",
        [id],
    )?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

pub fn set_task_missed(conn: &Connection, id: i64, at: Option<NaiveDateTime>) -> Result<()> {
    let changed = conn.execute(
        "UPDATE tarefas SET perdida_em = ?2 WHERE id = ?1",
        params![id, at],
    )?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

pub fn delete_task(conn: &Connection, id: i64) -> Result<()> {
    let changed = conn.execute("DELETE FROM tarefas WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

const OPEN_ORDER: &str = "ORDER BY data, hora IS NULL, hora, prioridade = 'alta' DESC, id";

pub fn today_view(conn: &Connection, today: NaiveDate) -> Result<TodayView> {
    Ok(TodayView {
        overdue: query_tasks(
            conn,
            &format!(
                "concluida_em IS NULL AND perdida_em IS NULL AND rotina_id IS NULL AND data < ?1 {OPEN_ORDER}"
            ),
            [today],
        )?,
        today: query_tasks(
            conn,
            &format!("concluida_em IS NULL AND data = ?1 {OPEN_ORDER}"),
            [today],
        )?,
        undated: query_tasks(
            conn,
            "concluida_em IS NULL AND data IS NULL ORDER BY prioridade = 'alta' DESC, id",
            [],
        )?,
    })
}

pub fn tasks_between(
    conn: &Connection,
    from: NaiveDate,
    to: NaiveDate,
    include_completed: bool,
) -> Result<Vec<Task>> {
    let completed = if include_completed {
        ""
    } else {
        "AND concluida_em IS NULL AND perdida_em IS NULL"
    };
    query_tasks(
        conn,
        &format!("data >= ?1 AND data <= ?2 {completed} {OPEN_ORDER}"),
        [from, to],
    )
}

pub fn upcoming(
    conn: &Connection,
    today: NaiveDate,
    days: i64,
) -> Result<Vec<(NaiveDate, Vec<Task>)>> {
    let tasks = tasks_between(
        conn,
        today + TimeDelta::days(1),
        today + TimeDelta::days(days),
        false,
    )?;
    let mut groups: Vec<(NaiveDate, Vec<Task>)> = Vec::new();
    for task in tasks {
        let Some(date) = task.date else { continue };
        match groups.last_mut() {
            Some((last, list)) if *last == date => list.push(task),
            _ => groups.push((date, vec![task])),
        }
    }
    Ok(groups)
}

pub fn pending_before(conn: &Connection, before: NaiveDate) -> Result<Vec<Task>> {
    query_tasks(
        conn,
        "concluida_em IS NULL AND perdida_em IS NULL AND rotina_id IS NULL AND (data IS NULL OR data < ?1)
         ORDER BY data IS NULL, data, hora IS NULL, hora, id",
        [before],
    )
}

pub fn completed(conn: &Connection, limit: usize) -> Result<Vec<Task>> {
    query_tasks(
        conn,
        "concluida_em IS NOT NULL ORDER BY concluida_em DESC, id DESC LIMIT ?1",
        [limit as i64],
    )
}

fn routine_from_row(row: &Row<'_>) -> rusqlite::Result<(Routine, String)> {
    let recurrence: String = row.get(3)?;
    Ok((
        Routine {
            id: row.get(0)?,
            title: row.get(1)?,
            notes: row.get(2)?,
            recurrence: Recurrence::Daily,
            time: row.get(4)?,
            active: row.get(5)?,
        },
        recurrence,
    ))
}

pub fn list_routines(conn: &Connection) -> Result<Vec<Routine>> {
    let mut stmt = conn.prepare(
        "SELECT id, titulo, notas, recorrencia, hora, ativa FROM rotinas ORDER BY hora IS NULL, hora, titulo",
    )?;
    let rows = stmt
        .query_map([], routine_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(mut routine, text)| {
            routine.recurrence = text.parse()?;
            Ok(routine)
        })
        .collect()
}

pub fn create_routine(conn: &Connection, routine: NewRoutine) -> Result<Routine> {
    let title = routine.title.trim();
    if title.is_empty() {
        return Err(AgendaError::EmptyTitle);
    }
    conn.execute(
        "INSERT INTO rotinas (titulo, notas, recorrencia, hora, ativa) VALUES (?1, ?2, ?3, ?4, 1)",
        params![
            title,
            routine.notes,
            routine.recurrence.to_string(),
            routine.time
        ],
    )?;
    let id = conn.last_insert_rowid();
    list_routines(conn)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or(AgendaError::NotFound(id))
}

pub fn set_routine_active(conn: &Connection, id: i64, active: bool) -> Result<()> {
    let changed = conn.execute(
        "UPDATE rotinas SET ativa = ?2 WHERE id = ?1",
        params![id, active],
    )?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

pub fn delete_routine(conn: &Connection, id: i64) -> Result<()> {
    let changed = conn.execute("DELETE FROM rotinas WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub date: NaiveDate,
    pub routine: Routine,
}

pub fn routine_projections(
    conn: &Connection,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> Result<Vec<Projection>> {
    let start = from.max(today + TimeDelta::days(1));
    let mut projections = Vec::new();
    if start > to {
        return Ok(projections);
    }
    for routine in list_routines(conn)?.into_iter().filter(|r| r.active) {
        for date in start.iter_days().take_while(|d| *d <= to) {
            if !routine.recurrence.occurs_on(date) {
                continue;
            }
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT id FROM tarefas WHERE rotina_id = ?1 AND data = ?2",
                    params![routine.id, date],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                projections.push(Projection {
                    date,
                    routine: routine.clone(),
                });
            }
        }
    }
    projections.sort_by_key(|p| (p.date, p.routine.time.is_some(), p.routine.time));
    Ok(projections)
}

pub fn get_routine(conn: &Connection, id: i64) -> Result<Routine> {
    list_routines(conn)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or(AgendaError::NotFound(id))
}

pub fn update_routine(conn: &Connection, id: i64, routine: NewRoutine) -> Result<Routine> {
    let title = routine.title.trim();
    if title.is_empty() {
        return Err(AgendaError::EmptyTitle);
    }
    let changed = conn.execute(
        "UPDATE rotinas SET titulo = ?2, notas = ?3, recorrencia = ?4, hora = ?5 WHERE id = ?1",
        params![
            id,
            title,
            routine.notes,
            routine.recurrence.to_string(),
            routine.time
        ],
    )?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    get_routine(conn, id)
}

pub fn move_task(conn: &Connection, id: i64, date: Option<NaiveDate>) -> Result<Task> {
    let task = get_task(conn, id)?;
    update_task(
        conn,
        id,
        NewTask {
            title: task.title,
            notes: task.notes,
            date,
            time: task.time,
            priority: task.priority,
        },
    )
}

pub fn move_reminder(
    conn: &Connection,
    id: i64,
    date: NaiveDate,
    now: NaiveDateTime,
) -> Result<Reminder> {
    let reminder = get_reminder(conn, id)?;
    let at = date.and_time(reminder.effective_time().time());
    update_reminder(conn, id, &reminder.title, &reminder.notes, at, now)
}

pub fn pending_reminders(conn: &Connection) -> Result<Vec<Reminder>> {
    query_reminders(
        conn,
        "l.disparado_em IS NULL ORDER BY COALESCE(l.adiado_ate, l.dispara_em), l.id",
        [],
    )
}

pub fn generate_routine_tasks(
    conn: &Connection,
    today: NaiveDate,
    now: NaiveDateTime,
) -> Result<usize> {
    let mut created = 0;
    for routine in list_routines(conn)? {
        if !routine.active || !routine.recurrence.occurs_on(today) {
            continue;
        }
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM tarefas WHERE rotina_id = ?1 AND data = ?2",
                params![routine.id, today],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            continue;
        }
        create_task_inner(
            conn,
            NewTask {
                title: routine.title.clone(),
                notes: routine.notes.clone(),
                date: Some(today),
                time: routine.time,
                priority: Priority::Normal,
            },
            Some(routine.id),
            now,
        )?;
        created += 1;
    }
    Ok(created)
}

fn insert_reminder(
    conn: &Connection,
    task_id: Option<i64>,
    title: Option<&str>,
    at: NaiveDateTime,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO lembretes (tarefa_id, titulo, dispara_em) VALUES (?1, ?2, ?3)",
        params![task_id, title, at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn create_reminder(
    conn: &Connection,
    task_id: Option<i64>,
    title: Option<&str>,
    at: NaiveDateTime,
) -> Result<Reminder> {
    if let Some(id) = task_id {
        get_task(conn, id)?;
    } else if title.is_none_or(|t| t.trim().is_empty()) {
        return Err(AgendaError::EmptyTitle);
    }
    let id = insert_reminder(conn, task_id, title.map(str::trim), at)?;
    get_reminder(conn, id)
}

const REMINDER_SELECT: &str = "SELECT l.id, l.tarefa_id, COALESCE(l.titulo, t.titulo, ''), l.dispara_em, l.disparado_em, l.adiado_ate, l.notas
     FROM lembretes l LEFT JOIN tarefas t ON t.id = l.tarefa_id";

fn reminder_from_row(row: &Row<'_>) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
        id: row.get(0)?,
        task_id: row.get(1)?,
        title: row.get(2)?,
        fires_at: row.get(3)?,
        fired_at: row.get(4)?,
        snoozed_until: row.get(5)?,
        notes: row.get(6)?,
    })
}

fn query_reminders(
    conn: &Connection,
    filter: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Reminder>> {
    let mut stmt = conn.prepare(&format!("{REMINDER_SELECT} WHERE {filter}"))?;
    let reminders = stmt
        .query_map(params, reminder_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(reminders)
}

pub fn get_reminder(conn: &Connection, id: i64) -> Result<Reminder> {
    query_reminders(conn, "l.id = ?1", [id])?
        .into_iter()
        .next()
        .ok_or(AgendaError::NotFound(id))
}

const PENDING: &str = "l.disparado_em IS NULL AND COALESCE(l.adiado_ate, l.dispara_em)";

pub fn due_reminders(conn: &Connection, now: NaiveDateTime) -> Result<Vec<Reminder>> {
    query_reminders(
        conn,
        &format!("{PENDING} <= ?1 ORDER BY COALESCE(l.adiado_ate, l.dispara_em), l.id"),
        [now],
    )
}

pub fn loose_reminders_between(
    conn: &Connection,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Reminder>> {
    let start = from.and_time(NaiveTime::MIN);
    let end = (to + TimeDelta::days(1)).and_time(NaiveTime::MIN);
    query_reminders(
        conn,
        "l.tarefa_id IS NULL
         AND COALESCE(l.adiado_ate, l.dispara_em) >= ?1
         AND COALESCE(l.adiado_ate, l.dispara_em) < ?2
         ORDER BY COALESCE(l.adiado_ate, l.dispara_em), l.id",
        params![start, end],
    )
}

pub fn update_reminder(
    conn: &Connection,
    id: i64,
    title: &str,
    notes: &str,
    at: NaiveDateTime,
    now: NaiveDateTime,
) -> Result<Reminder> {
    let title = title.trim();
    if title.is_empty() {
        return Err(AgendaError::EmptyTitle);
    }
    let current = get_reminder(conn, id)?;
    let moved = at != current.effective_time();
    let fired_at = if moved && at > now {
        None
    } else {
        current.fired_at
    };
    conn.execute(
        "UPDATE lembretes SET titulo = ?2, notas = ?3, dispara_em = ?4, adiado_ate = NULL, disparado_em = ?5
         WHERE id = ?1",
        params![id, title, notes, at, fired_at],
    )?;
    get_reminder(conn, id)
}

pub fn delete_reminder(conn: &Connection, id: i64) -> Result<()> {
    let changed = conn.execute("DELETE FROM lembretes WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

pub fn mark_reminder_fired(conn: &Connection, id: i64, now: NaiveDateTime) -> Result<()> {
    let changed = conn.execute(
        "UPDATE lembretes SET disparado_em = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

pub fn snooze_reminder(conn: &Connection, id: i64, until: NaiveDateTime) -> Result<()> {
    let changed = conn.execute(
        "UPDATE lembretes SET disparado_em = NULL, adiado_ate = ?2 WHERE id = ?1",
        params![id, until],
    )?;
    if changed == 0 {
        return Err(AgendaError::NotFound(id));
    }
    Ok(())
}

pub fn next_reminder_time(conn: &Connection) -> Result<Option<NaiveDateTime>> {
    let next = conn.query_row(
        "SELECT MIN(COALESCE(adiado_ate, dispara_em)) FROM lembretes WHERE disparado_em IS NULL",
        [],
        |row| row.get(0),
    )?;
    Ok(next)
}

pub fn take_missed_reminders(
    conn: &Connection,
    now: NaiveDateTime,
    grace: TimeDelta,
) -> Result<Vec<Reminder>> {
    let missed = query_reminders(
        conn,
        &format!("{PENDING} < ?1 ORDER BY COALESCE(l.adiado_ate, l.dispara_em), l.id"),
        [now - grace],
    )?;
    for reminder in &missed {
        mark_reminder_fired(conn, reminder.id, now)?;
    }
    Ok(missed)
}

#[cfg(test)]
mod tests;
