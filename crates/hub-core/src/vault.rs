use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::rand_core::{OsRng, RngCore};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::paths::HubPaths;

const VAULT_DIR: &str = "cofre";
const ENVELOPE_FILE: &str = "cofre.json";
const INDEX_FILE: &str = "indice.bin";
const ITEM_EXTENSION: &str = "bin";
const PARTIAL_SUFFIX: &str = ".parcial";
const TEMP_DIR: &str = "hub-cofre";
const FORMAT_VERSION: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const ID_BYTES: usize = 16;
const MAX_FILE_SIZE: u64 = 512 * 1024 * 1024;
const DEFAULT_MEMORY_KIB: u32 = 65536;
const DEFAULT_ITERATIONS: u32 = 3;
const DEFAULT_PARALLELISM: u32 = 1;
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("senha incorreta")]
    WrongPassword,
    #[error("o cofre já existe")]
    Exists,
    #[error("o cofre ainda não foi criado")]
    Missing,
    #[error("a senha não pode ficar vazia")]
    EmptyPassword,
    #[error("o arquivo passa do limite de 512 MB do cofre")]
    TooLarge,
    #[error("item não encontrado no cofre")]
    NotFound,
    #[error("erro de entrada e saída: {0}")]
    Io(#[from] std::io::Error),
    #[error("falha de criptografia: {0}")]
    Crypto(String),
    #[error("o cofre está em um formato inválido: {0}")]
    Format(String),
}

pub type Result<T> = std::result::Result<T, VaultError>;

#[derive(Clone)]
pub struct VaultKey(Arc<Zeroizing<[u8; KEY_LEN]>>);

impl VaultKey {
    fn new(bytes: Zeroizing<[u8; KEY_LEN]>) -> Self {
        Self(Arc::new(bytes))
    }

    fn bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VaultKey(..)")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VaultItem {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub added_at: NaiveDateTime,
}

#[derive(Debug, Clone, Copy)]
struct KdfParams {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            memory_kib: DEFAULT_MEMORY_KIB,
            iterations: DEFAULT_ITERATIONS,
            parallelism: DEFAULT_PARALLELISM,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    versao: u32,
    sal: String,
    m: u32,
    t: u32,
    p: u32,
    chave: String,
}

#[derive(Serialize, Deserialize)]
struct IndexEntry {
    id: String,
    nome: String,
    tamanho: u64,
    adicionado_em: String,
}

impl IndexEntry {
    fn into_item(self) -> Result<VaultItem> {
        let added_at = NaiveDateTime::parse_from_str(&self.adicionado_em, TIME_FORMAT)
            .map_err(|e| VaultError::Format(e.to_string()))?;
        Ok(VaultItem {
            id: self.id,
            name: self.nome,
            size: self.tamanho,
            added_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct VaultStore {
    dir: PathBuf,
    kdf: KdfParams,
}

impl VaultStore {
    pub fn new(paths: &HubPaths) -> Self {
        Self {
            dir: paths.data_dir.join(VAULT_DIR),
            kdf: KdfParams::default(),
        }
    }

    #[cfg(test)]
    fn with_params(paths: &HubPaths, memory_kib: u32, iterations: u32, parallelism: u32) -> Self {
        Self {
            kdf: KdfParams {
                memory_kib,
                iterations,
                parallelism,
            },
            ..Self::new(paths)
        }
    }

    pub fn exists(&self) -> bool {
        self.envelope_path().is_file()
    }

    pub fn create(&self, password: &str) -> Result<VaultKey> {
        if password.is_empty() {
            return Err(VaultError::EmptyPassword);
        }
        if self.exists() {
            return Err(VaultError::Exists);
        }
        std::fs::create_dir_all(&self.dir)?;
        let data_key = random_key();
        let envelope = wrap_key(password, &data_key, self.kdf)?;
        let key = VaultKey::new(data_key);
        self.write_index(&key, &[])?;
        self.write_envelope(&envelope)?;
        Ok(key)
    }

    pub fn unlock(&self, password: &str) -> Result<VaultKey> {
        let envelope = self.read_envelope()?;
        unwrap_key(password, &envelope).map(VaultKey::new)
    }

    pub fn change_password(&self, old: &str, new: &str) -> Result<()> {
        if new.is_empty() {
            return Err(VaultError::EmptyPassword);
        }
        let envelope = self.read_envelope()?;
        let data_key = unwrap_key(old, &envelope)?;
        let kdf = KdfParams {
            memory_kib: envelope.m,
            iterations: envelope.t,
            parallelism: envelope.p,
        };
        let rewrapped = wrap_key(new, &data_key, kdf)?;
        self.write_envelope(&rewrapped)
    }

    pub fn items(&self, key: &VaultKey) -> Result<Vec<VaultItem>> {
        let mut items = self
            .read_index(key)?
            .into_iter()
            .map(IndexEntry::into_item)
            .collect::<Result<Vec<_>>>()?;
        items.sort_by_key(|item| std::cmp::Reverse(item.added_at));
        Ok(items)
    }

    pub fn add_file(&self, key: &VaultKey, path: &Path, now: NaiveDateTime) -> Result<VaultItem> {
        let metadata = std::fs::metadata(path)?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(VaultError::TooLarge);
        }
        let mut entries = self.read_index(key)?;
        let content = Zeroizing::new(std::fs::read(path)?);
        let id = random_id();
        write_atomic(&self.item_path(&id), &seal(key.bytes(), &content)?)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "arquivo".to_string());
        let entry = IndexEntry {
            id,
            nome: name,
            tamanho: content.len() as u64,
            adicionado_em: now.format(TIME_FORMAT).to_string(),
        };
        entries.push(entry_clone(&entry));
        self.write_entries(key, &entries)?;
        entry.into_item()
    }

    pub fn rename(&self, key: &VaultKey, id: &str, name: &str) -> Result<()> {
        let mut entries = self.read_index(key)?;
        let entry = entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(VaultError::NotFound)?;
        entry.nome = name.to_string();
        self.write_entries(key, &entries)
    }

    pub fn remove(&self, key: &VaultKey, id: &str) -> Result<()> {
        let mut entries = self.read_index(key)?;
        let position = entries
            .iter()
            .position(|e| e.id == id)
            .ok_or(VaultError::NotFound)?;
        entries.remove(position);
        self.write_entries(key, &entries)?;
        match std::fs::remove_file(self.item_path(id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn export(&self, key: &VaultKey, id: &str, dest: &Path) -> Result<()> {
        let content = self.read_item(key, id)?;
        write_atomic(dest, &content)
    }

    pub fn open_temp(&self, key: &VaultKey, id: &str) -> Result<PathBuf> {
        let name = self
            .read_index(key)?
            .into_iter()
            .find(|e| e.id == id)
            .map(|e| e.nome)
            .ok_or(VaultError::NotFound)?;
        let content = self.read_item(key, id)?;
        let dir = temp_root().join(id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(safe_file_name(&name));
        std::fs::write(&path, &*content)?;
        Ok(path)
    }

    fn envelope_path(&self) -> PathBuf {
        self.dir.join(ENVELOPE_FILE)
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join(INDEX_FILE)
    }

    fn item_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ITEM_EXTENSION}"))
    }

    fn read_envelope(&self) -> Result<Envelope> {
        let bytes = match std::fs::read(self.envelope_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::Missing);
            }
            Err(error) => return Err(error.into()),
        };
        let envelope: Envelope =
            serde_json::from_slice(&bytes).map_err(|e| VaultError::Format(e.to_string()))?;
        if envelope.versao != FORMAT_VERSION {
            return Err(VaultError::Format(format!(
                "versão {} não suportada",
                envelope.versao
            )));
        }
        Ok(envelope)
    }

    fn write_envelope(&self, envelope: &Envelope) -> Result<()> {
        let json =
            serde_json::to_vec_pretty(envelope).map_err(|e| VaultError::Format(e.to_string()))?;
        write_atomic(&self.envelope_path(), &json)
    }

    fn read_index(&self, key: &VaultKey) -> Result<Vec<IndexEntry>> {
        let bytes = match std::fs::read(self.index_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return if self.exists() {
                    Ok(Vec::new())
                } else {
                    Err(VaultError::Missing)
                };
            }
            Err(error) => return Err(error.into()),
        };
        let plain = open(key.bytes(), &bytes)?;
        serde_json::from_slice(&plain).map_err(|e| VaultError::Format(e.to_string()))
    }

    fn write_entries(&self, key: &VaultKey, entries: &[IndexEntry]) -> Result<()> {
        let json = Zeroizing::new(
            serde_json::to_vec(entries).map_err(|e| VaultError::Format(e.to_string()))?,
        );
        write_atomic(&self.index_path(), &seal(key.bytes(), &json)?)
    }

    fn write_index(&self, key: &VaultKey, entries: &[IndexEntry]) -> Result<()> {
        self.write_entries(key, entries)
    }

    fn read_item(&self, key: &VaultKey, id: &str) -> Result<Zeroizing<Vec<u8>>> {
        if !self.read_index(key)?.iter().any(|e| e.id == id) {
            return Err(VaultError::NotFound);
        }
        let bytes = match std::fs::read(self.item_path(id)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(VaultError::NotFound);
            }
            Err(error) => return Err(error.into()),
        };
        open(key.bytes(), &bytes)
    }
}

pub fn clear_temp() {
    let _ = std::fs::remove_dir_all(temp_root());
}

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(TEMP_DIR)
}

fn entry_clone(entry: &IndexEntry) -> IndexEntry {
    IndexEntry {
        id: entry.id.clone(),
        nome: entry.nome.clone(),
        tamanho: entry.tamanho,
        adicionado_em: entry.adicionado_em.clone(),
    }
}

fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "arquivo".to_string()
    } else {
        cleaned
    }
}

fn random_key() -> Zeroizing<[u8; KEY_LEN]> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    OsRng.fill_bytes(&mut *key);
    key
}

fn random_id() -> String {
    let mut bytes = [0u8; ID_BYTES];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn derive(password: &str, salt: &[u8], kdf: KdfParams) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let params = Params::new(
        kdf.memory_kib,
        kdf.iterations,
        kdf.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|e| VaultError::Format(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password.as_bytes(), salt, &mut *out)
        .map_err(|e| VaultError::Crypto(e.to_string()))?;
    Ok(out)
}

fn seal(key: &[u8; KEY_LEN], plain: &[u8]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher
        .encrypt(XNonce::from_slice(&nonce), plain)
        .map_err(|_| VaultError::Crypto("não foi possível cifrar".to_string()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + encrypted.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&encrypted);
    Ok(out)
}

fn open(key: &[u8; KEY_LEN], data: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if data.len() < NONCE_LEN {
        return Err(VaultError::Format("arquivo cifrado truncado".to_string()));
    }
    let (nonce, encrypted) = data.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(XNonce::from_slice(nonce), encrypted)
        .map(Zeroizing::new)
        .map_err(|_| VaultError::Crypto("conteúdo adulterado ou chave inválida".to_string()))
}

fn wrap_key(password: &str, data_key: &[u8; KEY_LEN], kdf: KdfParams) -> Result<Envelope> {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let derived = derive(password, &salt, kdf)?;
    let wrapped = seal(&derived, data_key)?;
    Ok(Envelope {
        versao: FORMAT_VERSION,
        sal: STANDARD.encode(salt),
        m: kdf.memory_kib,
        t: kdf.iterations,
        p: kdf.parallelism,
        chave: STANDARD.encode(wrapped),
    })
}

fn unwrap_key(password: &str, envelope: &Envelope) -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let salt = STANDARD
        .decode(&envelope.sal)
        .map_err(|e| VaultError::Format(e.to_string()))?;
    let wrapped = STANDARD
        .decode(&envelope.chave)
        .map_err(|e| VaultError::Format(e.to_string()))?;
    let derived = derive(
        password,
        &salt,
        KdfParams {
            memory_kib: envelope.m,
            iterations: envelope.t,
            parallelism: envelope.p,
        },
    )?;
    let plain = open(&derived, &wrapped).map_err(|error| match error {
        VaultError::Crypto(_) => VaultError::WrongPassword,
        other => other,
    })?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    if plain.len() != KEY_LEN {
        return Err(VaultError::Format("chave de dados inválida".to_string()));
    }
    key.copy_from_slice(&plain);
    Ok(key)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut partial = OsString::from(path.as_os_str());
    partial.push(PARTIAL_SUFFIX);
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn setup() -> (tempfile::TempDir, VaultStore) {
        let temp = tempfile::tempdir().unwrap();
        let paths = HubPaths::from_home(temp.path().join("hub"));
        let store = VaultStore::with_params(&paths, 8, 1, 1);
        (temp, store)
    }

    fn source(temp: &tempfile::TempDir, name: &str, content: &[u8]) -> PathBuf {
        let path = temp.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn creates_and_unlocks() {
        let (_temp, store) = setup();
        assert!(!store.exists());
        assert!(matches!(store.unlock("x"), Err(VaultError::Missing)));
        let key = store.create("segredo").unwrap();
        assert!(store.exists());
        let again = store.unlock("segredo").unwrap();
        assert_eq!(key.bytes(), again.bytes());
        assert!(store.items(&again).unwrap().is_empty());
        assert_eq!(format!("{key:?}"), "VaultKey(..)");
    }

    #[test]
    fn rejects_empty_password_and_existing_vault() {
        let (_temp, store) = setup();
        assert!(matches!(store.create(""), Err(VaultError::EmptyPassword)));
        store.create("a").unwrap();
        assert!(matches!(store.create("b"), Err(VaultError::Exists)));
        assert!(store.unlock("a").is_ok());
    }

    #[test]
    fn wrong_password_is_rejected() {
        let (_temp, store) = setup();
        store.create("certa").unwrap();
        assert!(matches!(
            store.unlock("errada"),
            Err(VaultError::WrongPassword)
        ));
    }

    #[test]
    fn adds_lists_and_orders_newest_first() {
        let (temp, store) = setup();
        let key = store.create("s").unwrap();
        let first = source(&temp, "a.txt", b"um");
        let second = source(&temp, "b.txt", b"dois dois");
        let a = store
            .add_file(&key, &first, at("2026-10-21 09:00"))
            .unwrap();
        let b = store
            .add_file(&key, &second, at("2026-10-21 10:00"))
            .unwrap();
        assert_eq!(a.name, "a.txt");
        assert_eq!(a.size, 2);
        assert_eq!(a.id.len(), 32);
        let items = store.items(&key).unwrap();
        assert_eq!(items, vec![b, a]);
    }

    #[test]
    fn export_matches_original() {
        let (temp, store) = setup();
        let key = store.create("s").unwrap();
        let content: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        let item = store
            .add_file(
                &key,
                &source(&temp, "dados.bin", &content),
                at("2026-10-21 09:00"),
            )
            .unwrap();
        let dest = temp.path().join("saida.bin");
        store.export(&key, &item.id, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), content);
        assert!(matches!(
            store.export(&key, "inexistente", &dest),
            Err(VaultError::NotFound)
        ));
    }

    #[test]
    fn renames_item() {
        let (temp, store) = setup();
        let key = store.create("s").unwrap();
        let item = store
            .add_file(&key, &source(&temp, "a.txt", b"x"), at("2026-10-21 09:00"))
            .unwrap();
        store.rename(&key, &item.id, "novo.txt").unwrap();
        assert_eq!(store.items(&key).unwrap()[0].name, "novo.txt");
        assert!(matches!(
            store.rename(&key, "nada", "x"),
            Err(VaultError::NotFound)
        ));
    }

    #[test]
    fn removes_item_and_its_file() {
        let (temp, store) = setup();
        let key = store.create("s").unwrap();
        let item = store
            .add_file(&key, &source(&temp, "a.txt", b"x"), at("2026-10-21 09:00"))
            .unwrap();
        assert!(store.item_path(&item.id).is_file());
        store.remove(&key, &item.id).unwrap();
        assert!(!store.item_path(&item.id).exists());
        assert!(store.items(&key).unwrap().is_empty());
        assert!(matches!(
            store.remove(&key, &item.id),
            Err(VaultError::NotFound)
        ));
    }

    #[test]
    fn changing_password_keeps_items_readable() {
        let (temp, store) = setup();
        let key = store.create("antiga").unwrap();
        let item = store
            .add_file(
                &key,
                &source(&temp, "a.txt", b"conteudo"),
                at("2026-10-21 09:00"),
            )
            .unwrap();
        assert!(matches!(
            store.change_password("errada", "nova"),
            Err(VaultError::WrongPassword)
        ));
        assert!(matches!(
            store.change_password("antiga", ""),
            Err(VaultError::EmptyPassword)
        ));
        store.change_password("antiga", "nova").unwrap();
        assert!(matches!(
            store.unlock("antiga"),
            Err(VaultError::WrongPassword)
        ));
        let fresh = store.unlock("nova").unwrap();
        assert_eq!(store.items(&fresh).unwrap(), vec![item.clone()]);
        let dest = temp.path().join("fora.txt");
        store.export(&fresh, &item.id, &dest).unwrap();
        assert_eq!(std::fs::read(dest).unwrap(), b"conteudo");
    }

    #[test]
    fn disk_content_is_encrypted() {
        let (temp, store) = setup();
        let key = store.create("s").unwrap();
        let secret = b"TEXTO MUITO SECRETO DO COFRE";
        store
            .add_file(
                &key,
                &source(&temp, "segredo-visivel.txt", secret),
                at("2026-10-21 09:00"),
            )
            .unwrap();
        for entry in std::fs::read_dir(&store.dir).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("TEXTO MUITO SECRETO"));
            assert!(!text.contains("segredo-visivel"));
        }
    }

    #[test]
    fn open_temp_decrypts_to_named_file_and_clear_removes_it() {
        let (temp, store) = setup();
        let key = store.create("s").unwrap();
        let item = store
            .add_file(
                &key,
                &source(&temp, "nota.txt", b"ola"),
                at("2026-10-21 09:00"),
            )
            .unwrap();
        let path = store.open_temp(&key, &item.id).unwrap();
        assert_eq!(path.file_name().unwrap(), "nota.txt");
        assert_eq!(std::fs::read(&path).unwrap(), b"ola");
        clear_temp();
        assert!(!path.exists());
    }

    #[test]
    fn rejects_files_above_limit() {
        let (temp, store) = setup();
        let key = store.create("s").unwrap();
        let path = temp.path().join("grande.bin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE_SIZE + 1).unwrap();
        assert!(matches!(
            store.add_file(&key, &path, at("2026-10-21 09:00")),
            Err(VaultError::TooLarge)
        ));
    }

    #[test]
    fn sanitizes_file_names() {
        assert_eq!(safe_file_name("a/b:c.txt"), "a_b_c.txt");
        assert_eq!(safe_file_name(".."), "arquivo");
    }
}
