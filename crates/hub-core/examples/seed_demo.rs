use hub_core::agenda::{self, NewRoutine, Recurrence, quick_add};
use hub_core::chrono::{Local, NaiveTime, TimeDelta};
use hub_core::{Database, HubPaths};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = HubPaths::resolve()?;
    paths.ensure_dirs()?;
    let db = Database::open(&paths.database_file())?;
    let conn = db.conn();
    let now = Local::now().naive_local();
    let today = now.date();
    let day = |offset: i64| today + TimeDelta::days(offset);
    let time = |h: u32, m: u32| NaiveTime::from_hms_opt(h, m, 0);

    let appointments = [
        ("Pagar conta de luz", Some(day(-12)), None, false),
        ("Enviar relatório", Some(day(-2)), time(16, 0), true),
        ("Revisar orçamento", Some(day(-1)), None, false),
        ("Ligar para o dentista", Some(day(0)), None, false),
        ("Reunião do projeto", Some(day(0)), time(15, 30), false),
        ("Entregar proposta", Some(day(1)), time(10, 0), false),
        ("Almoço com a Ana", Some(day(2)), time(12, 30), false),
        ("Consulta médica", Some(day(4)), time(8, 30), false),
        ("Aniversário do Pedro", Some(day(6)), None, false),
        ("Comprar pão", None, None, false),
    ];
    for (title, date, at, done) in appointments {
        let task = agenda::create_task(
            conn,
            agenda::NewTask {
                title: title.into(),
                date,
                time: at,
                ..agenda::NewTask::default()
            },
            now,
        )?;
        if done {
            agenda::complete_task(conn, task.id, now)?;
        }
    }
    agenda::create_task(
        conn,
        quick_add::parse("pagar boleto do cartão depois de amanhã !alta", now).into(),
        now,
    )?;

    for (title, offset, h, m) in [
        ("Beber água", 0, 17, 0),
        ("Tirar o lixo", 1, 20, 0),
        ("Renovar assinatura", 3, 9, 0),
    ] {
        if let Some(at) = time(h, m) {
            agenda::create_reminder(conn, None, Some(title), day(offset).and_time(at))?;
        }
    }

    for (title, recurrence, at) in [
        ("Tomar remédio", Recurrence::Daily, time(8, 0)),
        ("Academia", "semana:1,3,5".parse()?, time(7, 0)),
    ] {
        let routine = agenda::create_routine(
            conn,
            NewRoutine {
                title: title.into(),
                notes: String::new(),
                recurrence,
                time: at,
            },
        )?;
        for offset in -3..=7 {
            if routine.recurrence.occurs_on(day(offset)) && offset <= 0 {
                agenda::create_task(
                    conn,
                    agenda::NewTask {
                        title: routine.title.clone(),
                        date: Some(day(offset)),
                        time: routine.time,
                        ..agenda::NewTask::default()
                    },
                    now,
                )?;
            }
        }
    }
    conn.execute(
        "UPDATE tarefas SET rotina_id = (SELECT id FROM rotinas WHERE rotinas.titulo = tarefas.titulo)
         WHERE titulo IN (SELECT titulo FROM rotinas)",
        [],
    )?;
    agenda::take_missed_reminders(conn, now, TimeDelta::minutes(2))?;
    println!(
        "dados de exemplo gravados em {}",
        paths.database_file().display()
    );
    Ok(())
}
