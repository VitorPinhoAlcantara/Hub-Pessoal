use std::sync::Arc;

use hub_core::clock::FakeClock;
use hub_core::platform::FakePlatform;
use hub_core::{Config, Database, HubContext, HubPaths};

use super::*;

fn setup() -> (Notes, HubContext, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let paths = HubPaths::from_home(temp.path());
    paths.ensure_dirs().unwrap();
    let ctx = HubContext::new(
        paths,
        Config::default(),
        Database::open_in_memory().unwrap(),
        Arc::new(FakePlatform::default()),
        Arc::new(FakeClock::at("2026-10-21 10:00")),
    );
    (Notes::new(ctx.clone()), ctx, temp)
}

fn send(section: &mut Notes, messages: impl IntoIterator<Item = Message>) {
    for message in messages {
        let _ = section.update(message);
    }
}

fn type_body(section: &mut Notes, text: &str) {
    let action = text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(text.to_string())));
    send(section, [Message::BodyAction(action)]);
}

#[test]
fn creates_edits_and_saves_note() {
    let (mut section, ctx, _temp) = setup();
    send(&mut section, [Message::NewNote]);
    assert_eq!(section.notes().len(), 1);
    let id = section.editing().unwrap().id;
    send(&mut section, [Message::TitleChanged("Compras".into())]);
    type_body(&mut section, "leite e pão");
    assert!(section.is_dirty());
    section.save_pending();
    assert!(!section.is_dirty());
    let stored = notes::get(ctx.db().conn(), id).unwrap().unwrap();
    assert_eq!(stored.title, "Compras");
    assert_eq!(stored.body.trim(), "leite e pão");
    let _ = section.view();
}

#[test]
fn autosave_waits_for_the_delay() {
    let (mut section, ctx, _temp) = setup();
    send(&mut section, [Message::NewNote]);
    let id = section.editing().unwrap().id;
    send(&mut section, [Message::TitleChanged("A".into())]);
    send(&mut section, [Message::Autosave]);
    assert!(section.is_dirty());
    section.dirty_since = Instant::now().checked_sub(AUTOSAVE_DELAY * 2);
    send(&mut section, [Message::Autosave]);
    assert!(!section.is_dirty());
    assert_eq!(notes::get(ctx.db().conn(), id).unwrap().unwrap().title, "A");
}

#[test]
fn switching_note_saves_the_previous_one() {
    let (mut section, ctx, _temp) = setup();
    send(&mut section, [Message::NewNote]);
    let first = section.editing().unwrap().id;
    send(&mut section, [Message::TitleChanged("Primeira".into())]);
    send(&mut section, [Message::NewNote]);
    assert_ne!(section.editing().unwrap().id, first);
    assert_eq!(
        notes::get(ctx.db().conn(), first).unwrap().unwrap().title,
        "Primeira"
    );
}

#[test]
fn favorite_moves_note_to_top() {
    let (mut section, ctx, _temp) = setup();
    let a = notes::create(ctx.db().conn(), "A", "", ctx.clock.now()).unwrap();
    let b = notes::create(ctx.db().conn(), "B", "", ctx.clock.now()).unwrap();
    section.refresh();
    send(&mut section, [Message::FavoriteOf(a.id)]);
    assert_eq!(section.notes()[0].id, a.id);
    assert!(section.notes()[0].favorite);
    send(
        &mut section,
        [Message::FavoriteOf(a.id), Message::FavoriteOf(b.id)],
    );
    assert_eq!(section.notes()[0].id, b.id);
    let _ = section.view();
}

#[test]
fn changes_reset_type() {
    let (mut section, _ctx, _temp) = setup();
    send(&mut section, [Message::NewNote]);
    send(&mut section, [Message::ResetSelected(TempReset::Daily)]);
    assert_eq!(section.selected().unwrap().reset, TempReset::Daily);
    let _ = section.view();
}

#[test]
fn delete_requires_confirmation() {
    let (mut section, _ctx, _temp) = setup();
    send(&mut section, [Message::NewNote]);
    let id = section.editing().unwrap().id;
    send(&mut section, [Message::AskDeleteNote(id)]);
    assert!(matches!(section.modal(), Some(Modal::ConfirmDeleteNote(_))));
    let _ = section.view();
    send(&mut section, [Message::ModalClosed]);
    assert_eq!(section.notes().len(), 1);
    send(
        &mut section,
        [Message::AskDeleteNote(id), Message::DeleteConfirmed],
    );
    assert!(section.notes().is_empty());
    assert!(section.editing().is_none());
    let _ = section.view();
}

#[test]
fn search_filters_notes() {
    let (mut section, ctx, _temp) = setup();
    notes::create(
        ctx.db().conn(),
        "Receita de bolo",
        "farinha",
        ctx.clock.now(),
    )
    .unwrap();
    notes::create(ctx.db().conn(), "Reunião", "pauta", ctx.clock.now()).unwrap();
    section.refresh();
    assert_eq!(section.notes().len(), 2);
    send(&mut section, [Message::QueryChanged("bolo".into())]);
    assert_eq!(section.notes().len(), 1);
    assert_eq!(section.notes()[0].title, "Receita de bolo");
    let _ = section.view();
}

#[test]
fn refresh_keeps_selection() {
    let (mut section, ctx, _temp) = setup();
    send(&mut section, [Message::NewNote]);
    let id = section.editing().unwrap().id;
    notes::update(ctx.db().conn(), id, "Externa", "texto", ctx.clock.now()).unwrap();
    section.refresh();
    assert_eq!(section.editing().unwrap().id, id);
    assert_eq!(section.editing().unwrap().title, "Externa");
}

#[test]
fn vault_creation_needs_matching_passwords() {
    let (mut section, _ctx, _temp) = setup();
    send(&mut section, [Message::TabSelected(Tab::Vault)]);
    assert!(!section.vault_exists());
    let _ = section.view();
    send(
        &mut section,
        [
            Message::PasswordChanged("segredo".into()),
            Message::ConfirmChanged("outro".into()),
        ],
    );
    assert!(!section.can_create());
    send(&mut section, [Message::CreateVault]);
    assert!(section.busy().is_none());
    send(&mut section, [Message::ConfirmChanged("segredo".into())]);
    assert!(section.can_create());
    let _ = section.view();
}

#[test]
fn vault_flow_wrong_password_unlock_and_lock() {
    let (mut section, ctx, temp) = setup();
    let store = VaultStore::new(&ctx.paths);
    store.create("segredo").unwrap();
    send(&mut section, [Message::TabSelected(Tab::Vault)]);
    assert!(section.vault_exists());
    assert!(!section.vault_unlocked());
    let _ = section.view();

    let wrong = store.unlock("errada").unwrap_err();
    send(
        &mut section,
        [Message::VaultOpened(Err(Failure::from(wrong)))],
    );
    assert!(section.wrong_password());
    assert!(!section.vault_unlocked());
    let _ = section.view();

    let key = store.unlock("segredo").unwrap();
    let file = temp.path().join("doc.txt");
    std::fs::write(&file, "conteudo").unwrap();
    store.add_file(&key, &file, ctx.clock.now()).unwrap();
    let items = store.items(&key).unwrap();
    send(&mut section, [Message::VaultOpened(Ok((key, items)))]);
    assert!(section.vault_unlocked());
    assert!(!section.wrong_password());
    assert_eq!(section.vault_items().len(), 1);
    let _ = section.view();

    let id = section.vault_items()[0].id.clone();
    send(&mut section, [Message::AskRename(id.clone())]);
    let _ = section.view();
    send(
        &mut section,
        [
            Message::RenameChanged("novo.txt".into()),
            Message::RenameConfirmed,
        ],
    );
    assert_eq!(section.vault_items()[0].name, "novo.txt");

    send(&mut section, [Message::AskRemove(id.clone())]);
    let _ = section.view();
    send(&mut section, [Message::RemoveConfirmed]);
    assert!(section.vault_items().is_empty());

    send(&mut section, [Message::OpenChangePassword]);
    let _ = section.view();
    send(&mut section, [Message::ModalClosed]);

    section.vault_activity = Instant::now().checked_sub(VAULT_IDLE * 2).unwrap();
    send(&mut section, [Message::VaultTick]);
    assert!(!section.vault_unlocked());
    assert!(section.vault_items().is_empty());
    let _ = section.view();
}

#[test]
fn hiding_locks_vault_and_saves_note() {
    let (mut section, ctx, _temp) = setup();
    let store = VaultStore::new(&ctx.paths);
    let key = store.create("segredo").unwrap();
    send(&mut section, [Message::NewNote]);
    let id = section.editing().unwrap().id;
    send(&mut section, [Message::TitleChanged("Salva".into())]);
    send(&mut section, [Message::VaultOpened(Ok((key, Vec::new())))]);
    assert!(section.vault_unlocked());
    section.on_hide();
    assert!(!section.vault_unlocked());
    assert_eq!(
        notes::get(ctx.db().conn(), id).unwrap().unwrap().title,
        "Salva"
    );
}
