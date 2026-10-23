use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub type Result<T> = rusqlite::Result<T>;

const TITLE_PREVIEW: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TempReset {
    #[default]
    None,
    Daily,
    Weekly,
}

impl TempReset {
    pub const ALL: [TempReset; 3] = [TempReset::None, TempReset::Daily, TempReset::Weekly];

    pub fn label(self) -> &'static str {
        match self {
            TempReset::None => "Permanente",
            TempReset::Daily => "Diária",
            TempReset::Weekly => "Semanal",
        }
    }

    fn as_db(self) -> &'static str {
        match self {
            TempReset::None => "",
            TempReset::Daily => "diaria",
            TempReset::Weekly => "semanal",
        }
    }

    fn from_db(text: &str) -> Self {
        match text {
            "diaria" => TempReset::Daily,
            "semanal" => TempReset::Weekly,
            _ => TempReset::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub favorite: bool,
    pub reset: TempReset,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub last_reset: Option<NaiveDateTime>,
}

impl Note {
    pub fn display_title(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_string();
        }
        self.body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.chars().take(TITLE_PREVIEW).collect())
            .unwrap_or_else(|| "Nota sem título".to_string())
    }
}

const COLUMNS: &str =
    "id, titulo, conteudo, favorita, reset, criada_em, atualizada_em, ultimo_reset";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        favorite: row.get::<_, i64>(3)? != 0,
        reset: TempReset::from_db(&row.get::<_, String>(4)?),
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        last_reset: row.get(7)?,
    })
}

pub fn list(conn: &Connection) -> Result<Vec<Note>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM notas ORDER BY favorita DESC, atualizada_em DESC, id DESC"
    ))?;
    stmt.query_map([], from_row)?.collect()
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Note>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM notas WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()
}

pub fn create(conn: &Connection, title: &str, body: &str, now: NaiveDateTime) -> Result<Note> {
    conn.execute(
        "INSERT INTO notas (titulo, conteudo, criada_em, atualizada_em) VALUES (?1, ?2, ?3, ?3)",
        params![title, body, now],
    )?;
    let id = conn.last_insert_rowid();
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM notas WHERE id = ?1"),
        [id],
        from_row,
    )
}

pub fn update(
    conn: &Connection,
    id: i64,
    title: &str,
    body: &str,
    now: NaiveDateTime,
) -> Result<()> {
    conn.execute(
        "UPDATE notas SET titulo = ?2, conteudo = ?3, atualizada_em = ?4 WHERE id = ?1",
        params![id, title, body, now],
    )?;
    Ok(())
}

pub fn set_favorite(conn: &Connection, id: i64, favorite: bool) -> Result<()> {
    conn.execute(
        "UPDATE notas SET favorita = ?2 WHERE id = ?1",
        params![id, favorite as i64],
    )?;
    Ok(())
}

pub fn set_reset(conn: &Connection, id: i64, reset: TempReset, now: NaiveDateTime) -> Result<()> {
    let last_reset = (reset != TempReset::None).then_some(now);
    conn.execute(
        "UPDATE notas SET reset = ?2, ultimo_reset = ?3 WHERE id = ?1",
        params![id, reset.as_db(), last_reset],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM notas WHERE id = ?1", [id])?;
    Ok(())
}

pub fn favorite(conn: &Connection) -> Result<Option<Note>> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM notas WHERE favorita = 1 ORDER BY atualizada_em DESC, id DESC LIMIT 1"
        ),
        [],
        from_row,
    )
    .optional()
}

pub fn search(conn: &Connection, query: &str) -> Result<Vec<Note>> {
    let needle = query.trim().to_lowercase();
    Ok(list(conn)?
        .into_iter()
        .filter(|note| {
            note.title.to_lowercase().contains(&needle)
                || note.body.to_lowercase().contains(&needle)
        })
        .collect())
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - TimeDelta::days(date.weekday().num_days_from_monday() as i64)
}

fn is_due(note: &Note, now: NaiveDateTime) -> bool {
    let last = note.last_reset.unwrap_or(note.created_at);
    match note.reset {
        TempReset::None => false,
        TempReset::Daily => last.date() < now.date(),
        TempReset::Weekly => monday_of(last.date()) < monday_of(now.date()),
    }
}

pub fn reset_due(conn: &Connection, now: NaiveDateTime) -> Result<usize> {
    let mut cleared = 0;
    for note in list(conn)? {
        if !is_due(&note, now) {
            continue;
        }
        conn.execute(
            "UPDATE notas SET conteudo = '', ultimo_reset = ?2 WHERE id = ?1",
            params![note.id, now],
        )?;
        cleared += 1;
    }
    Ok(cleared)
}

pub fn next_reset(conn: &Connection, now: NaiveDateTime) -> Result<Option<NaiveDateTime>> {
    let count = |kind: TempReset| -> Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM notas WHERE reset = ?1",
            [kind.as_db()],
            |row| row.get(0),
        )
    };
    if count(TempReset::Daily)? > 0 {
        let midnight = (now.date() + TimeDelta::days(1)).and_time(NaiveTime::MIN);
        return Ok(Some(midnight));
    }
    if count(TempReset::Weekly)? > 0 {
        let days = 7 - now.date().weekday().num_days_from_monday() as i64;
        let monday = (now.date() + TimeDelta::days(days)).and_time(NaiveTime::MIN);
        return Ok(Some(monday));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn creates_and_reads_notes() {
        let db = db();
        let note = create(db.conn(), "Compras", "leite", at("2026-10-21 09:00")).unwrap();
        assert_eq!(note.title, "Compras");
        assert_eq!(note.body, "leite");
        assert!(!note.favorite);
        assert_eq!(note.reset, TempReset::None);
        assert_eq!(get(db.conn(), note.id).unwrap(), Some(note));
        assert_eq!(get(db.conn(), 999).unwrap(), None);
    }

    #[test]
    fn edits_note_and_touches_update_time() {
        let db = db();
        let note = create(db.conn(), "a", "b", at("2026-10-21 09:00")).unwrap();
        update(db.conn(), note.id, "novo", "corpo", at("2026-10-21 10:00")).unwrap();
        let edited = get(db.conn(), note.id).unwrap().unwrap();
        assert_eq!(edited.title, "novo");
        assert_eq!(edited.body, "corpo");
        assert_eq!(edited.created_at, at("2026-10-21 09:00"));
        assert_eq!(edited.updated_at, at("2026-10-21 10:00"));
    }

    #[test]
    fn favorite_comes_first_and_is_found() {
        let db = db();
        let old = create(db.conn(), "velha", "", at("2026-10-21 08:00")).unwrap();
        let new = create(db.conn(), "nova", "", at("2026-10-21 09:00")).unwrap();
        let titles = |notes: Vec<Note>| notes.into_iter().map(|n| n.title).collect::<Vec<_>>();
        assert_eq!(titles(list(db.conn()).unwrap()), ["nova", "velha"]);
        assert_eq!(favorite(db.conn()).unwrap(), None);
        set_favorite(db.conn(), old.id, true).unwrap();
        assert_eq!(titles(list(db.conn()).unwrap()), ["velha", "nova"]);
        assert_eq!(favorite(db.conn()).unwrap().unwrap().id, old.id);
        set_favorite(db.conn(), new.id, true).unwrap();
        update(db.conn(), old.id, "velha", "x", at("2026-10-21 12:00")).unwrap();
        assert_eq!(favorite(db.conn()).unwrap().unwrap().id, old.id);
        set_favorite(db.conn(), old.id, false).unwrap();
        assert_eq!(favorite(db.conn()).unwrap().unwrap().id, new.id);
    }

    #[test]
    fn search_matches_title_or_body_ignoring_case() {
        let db = db();
        create(
            db.conn(),
            "Receita",
            "farinha e ovos",
            at("2026-10-21 08:00"),
        )
        .unwrap();
        create(db.conn(), "Ideias", "app novo", at("2026-10-21 09:00")).unwrap();
        assert_eq!(search(db.conn(), "RECEITA").unwrap().len(), 1);
        assert_eq!(search(db.conn(), "Farinha").unwrap().len(), 1);
        assert_eq!(search(db.conn(), "e").unwrap().len(), 2);
        assert!(search(db.conn(), "zzz").unwrap().is_empty());
    }

    #[test]
    fn deletes_note() {
        let db = db();
        let note = create(db.conn(), "a", "b", at("2026-10-21 08:00")).unwrap();
        delete(db.conn(), note.id).unwrap();
        assert!(list(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn display_title_falls_back_to_body_then_placeholder() {
        let db = db();
        let mut note = create(db.conn(), "", "", at("2026-10-21 08:00")).unwrap();
        assert_eq!(note.display_title(), "Nota sem título");
        note.body = "\n\n  primeira linha  \nsegunda".into();
        assert_eq!(note.display_title(), "primeira linha");
        note.body = "x".repeat(100);
        assert_eq!(note.display_title().chars().count(), 60);
        note.title = "Meu título".into();
        assert_eq!(note.display_title(), "Meu título");
    }

    #[test]
    fn daily_reset_clears_body_after_midnight_only() {
        let db = db();
        let note = create(db.conn(), "Hoje", "fazer x", at("2026-10-21 08:00")).unwrap();
        set_reset(db.conn(), note.id, TempReset::Daily, at("2026-10-21 08:00")).unwrap();
        assert_eq!(reset_due(db.conn(), at("2026-10-21 23:59")).unwrap(), 0);
        assert_eq!(get(db.conn(), note.id).unwrap().unwrap().body, "fazer x");
        assert_eq!(reset_due(db.conn(), at("2026-10-22 00:00")).unwrap(), 1);
        let cleared = get(db.conn(), note.id).unwrap().unwrap();
        assert_eq!(cleared.body, "");
        assert_eq!(cleared.title, "Hoje");
        assert_eq!(cleared.last_reset, Some(at("2026-10-22 00:00")));
        assert_eq!(reset_due(db.conn(), at("2026-10-22 18:00")).unwrap(), 0);
    }

    #[test]
    fn weekly_reset_happens_on_monday() {
        let db = db();
        let note = create(db.conn(), "Semana", "metas", at("2026-10-21 08:00")).unwrap();
        set_reset(
            db.conn(),
            note.id,
            TempReset::Weekly,
            at("2026-10-21 08:00"),
        )
        .unwrap();
        assert_eq!(reset_due(db.conn(), at("2026-10-25 23:59")).unwrap(), 0);
        assert_eq!(reset_due(db.conn(), at("2026-10-26 00:00")).unwrap(), 1);
        assert_eq!(get(db.conn(), note.id).unwrap().unwrap().body, "");
        assert_eq!(reset_due(db.conn(), at("2026-11-01 12:00")).unwrap(), 0);
    }

    #[test]
    fn permanent_notes_are_never_reset_and_none_clears_marker() {
        let db = db();
        let note = create(db.conn(), "a", "b", at("2026-10-21 08:00")).unwrap();
        assert_eq!(reset_due(db.conn(), at("2026-12-01 00:00")).unwrap(), 0);
        set_reset(db.conn(), note.id, TempReset::Daily, at("2026-10-21 08:00")).unwrap();
        set_reset(db.conn(), note.id, TempReset::None, at("2026-10-21 09:00")).unwrap();
        let stored = get(db.conn(), note.id).unwrap().unwrap();
        assert_eq!(stored.reset, TempReset::None);
        assert_eq!(stored.last_reset, None);
    }

    #[test]
    fn next_reset_prefers_daily_then_weekly() {
        let db = db();
        let now = at("2026-10-21 09:00");
        assert_eq!(next_reset(db.conn(), now).unwrap(), None);
        let weekly = create(db.conn(), "w", "", now).unwrap();
        set_reset(db.conn(), weekly.id, TempReset::Weekly, now).unwrap();
        assert_eq!(
            next_reset(db.conn(), now).unwrap(),
            Some(at("2026-10-26 00:00"))
        );
        assert_eq!(
            next_reset(db.conn(), at("2026-10-26 10:00")).unwrap(),
            Some(at("2026-11-02 00:00"))
        );
        let daily = create(db.conn(), "d", "", now).unwrap();
        set_reset(db.conn(), daily.id, TempReset::Daily, now).unwrap();
        assert_eq!(
            next_reset(db.conn(), now).unwrap(),
            Some(at("2026-10-22 00:00"))
        );
    }

    #[test]
    fn labels_and_all_cover_every_variant() {
        let labels: Vec<_> = TempReset::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(labels, ["Permanente", "Diária", "Semanal"]);
        assert_eq!(TempReset::default(), TempReset::None);
    }
}
