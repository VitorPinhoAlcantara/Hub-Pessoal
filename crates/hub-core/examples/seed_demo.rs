use hub_core::agenda::{self, NewRoutine, Recurrence, quick_add};
use hub_core::chrono::{Local, NaiveTime, TimeDelta};
use hub_core::{Database, HubPaths};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = HubPaths::resolve()?;
    paths.ensure_dirs()?;
    let db = Database::open(&paths.database_file())?;
    let conn = db.conn();
    let now = Local::now().naive_local();
    for text in [
        "pagar conta de luz ontem",
        "revisar orçamento hoje 17h !alta",
        "ligar para o dentista hoje",
        "comprar pão",
        "ler capítulo 3",
        "reunião do projeto amanhã 10h",
        "academia sexta 7h",
        "aniversário da Ana dia 30",
    ] {
        let parsed = quick_add::parse(text, now);
        let mut task: agenda::NewTask = parsed.into();
        if text.ends_with("ontem") {
            task.title = "pagar conta de luz".into();
            task.date = Some(now.date() - TimeDelta::days(1));
        }
        agenda::create_task(conn, task, now)?;
    }
    let done = agenda::create_task(
        conn,
        quick_add::parse("enviar relatório hoje", now).into(),
        now,
    )?;
    agenda::complete_task(conn, done.id, now)?;
    agenda::create_routine(
        conn,
        NewRoutine {
            title: "Tomar remédio".into(),
            notes: String::new(),
            recurrence: Recurrence::Daily,
            time: NaiveTime::from_hms_opt(8, 0, 0),
        },
    )?;
    agenda::create_routine(
        conn,
        NewRoutine {
            title: "Regar as plantas".into(),
            notes: String::new(),
            recurrence: "semana:1,4".parse()?,
            time: None,
        },
    )?;
    agenda::generate_routine_tasks(conn, now.date(), now)?;
    println!(
        "dados de exemplo gravados em {}",
        paths.database_file().display()
    );
    Ok(())
}
