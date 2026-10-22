use chrono::NaiveDateTime;
use rusqlite::{Connection, params};

pub const FAVORITES_LIMIT: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Favorite {
    pub url: String,
    pub title: String,
    pub used_at: NaiveDateTime,
}

impl Favorite {
    pub fn label(&self) -> &str {
        if self.title.trim().is_empty() {
            &self.url
        } else {
            &self.title
        }
    }

    pub fn is_playlist(&self) -> bool {
        is_playlist(&self.url)
    }
}

pub fn is_playlist(url: &str) -> bool {
    url.contains("list=")
}

pub fn is_supported_link(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("https://") || url.starts_with("http://")
}

pub fn touch_favorite(conn: &Connection, url: &str, now: NaiveDateTime) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO player_favoritos (url, usado_em) VALUES (?1, ?2)
         ON CONFLICT(url) DO UPDATE SET usado_em = excluded.usado_em",
        params![url.trim(), now],
    )?;
    Ok(())
}

pub fn name_favorite(conn: &Connection, url: &str, title: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE player_favoritos SET titulo = ?2 WHERE url = ?1 AND titulo = ''",
        params![url.trim(), title.trim()],
    )?;
    Ok(())
}

pub fn favorites(conn: &Connection) -> rusqlite::Result<Vec<Favorite>> {
    let mut stmt = conn.prepare(
        "SELECT url, titulo, usado_em FROM player_favoritos ORDER BY usado_em DESC, url LIMIT ?1",
    )?;
    stmt.query_map([FAVORITES_LIMIT as i64], |row| {
        Ok(Favorite {
            url: row.get(0)?,
            title: row.get(1)?,
            used_at: row.get(2)?,
        })
    })?
    .collect()
}

pub fn remove_favorite(conn: &Connection, url: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM player_favoritos WHERE url = ?1", [url])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn favorites_are_ordered_by_last_use_and_named_once() {
        let db = Database::open_in_memory().unwrap();
        let c = db.conn();
        touch_favorite(c, "https://youtu.be/a", at("2026-10-21 10:00")).unwrap();
        touch_favorite(
            c,
            "https://youtube.com/playlist?list=b",
            at("2026-10-21 11:00"),
        )
        .unwrap();
        name_favorite(c, "https://youtu.be/a", "Música A").unwrap();
        name_favorite(c, "https://youtu.be/a", "Outro nome").unwrap();
        touch_favorite(c, "https://youtu.be/a", at("2026-10-21 12:00")).unwrap();
        let list = favorites(c).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].label(), "Música A");
        assert!(list[1].is_playlist());
        assert_eq!(list[1].label(), "https://youtube.com/playlist?list=b");
        remove_favorite(c, "https://youtu.be/a").unwrap();
        assert_eq!(favorites(c).unwrap().len(), 1);
    }

    #[test]
    fn favorites_are_limited() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..20 {
            touch_favorite(
                db.conn(),
                &format!("https://x/{i}"),
                at("2026-10-21 10:00") + chrono::TimeDelta::minutes(i),
            )
            .unwrap();
        }
        let list = favorites(db.conn()).unwrap();
        assert_eq!(list.len(), FAVORITES_LIMIT);
        assert_eq!(list[0].url, "https://x/19");
    }

    #[test]
    fn links_are_validated() {
        assert!(is_supported_link(" https://youtu.be/x "));
        assert!(!is_supported_link("youtu.be/x"));
    }
}
