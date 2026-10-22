use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("pasta crates")
        .to_path_buf()
}

fn dependencies(manifest: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(manifest).expect("Cargo.toml legível");
    let mut in_dependencies = false;
    let mut names = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_dependencies = line.ends_with("dependencies]");
            continue;
        }
        if in_dependencies && let Some((name, _)) = line.split_once(['=', '.']) {
            names.push(name.trim().to_string());
        }
    }
    names
}

fn crate_names() -> Vec<(String, PathBuf)> {
    std::fs::read_dir(crates_dir())
        .expect("pasta crates legível")
        .flatten()
        .map(|entry| entry.path().join("Cargo.toml"))
        .filter(|manifest| manifest.exists())
        .map(|manifest| {
            let name = manifest
                .parent()
                .and_then(|dir| dir.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name, manifest)
        })
        .collect()
}

fn is_tab_or_panel(name: &str) -> bool {
    name.starts_with("section-") || name.starts_with("panel-")
}

#[test]
fn sections_and_panels_do_not_depend_on_each_other() {
    for (name, manifest) in crate_names()
        .into_iter()
        .filter(|(n, _)| is_tab_or_panel(n))
    {
        for dependency in dependencies(&manifest) {
            assert!(
                !is_tab_or_panel(&dependency),
                "{name} depende de {dependency}"
            );
            assert!(
                dependency != "hub-app" && dependency != "hub-mcp",
                "{name} depende de {dependency}"
            );
        }
    }
}

#[test]
fn core_does_not_depend_on_iced() {
    let manifest = crates_dir().join("hub-core").join("Cargo.toml");
    let deps = dependencies(&manifest);
    assert!(!deps.iter().any(|d| d.starts_with("iced")), "{deps:?}");
}

#[test]
fn target_os_cfg_only_inside_platform() {
    let mut offenders = Vec::new();
    let mut pending = vec![crates_dir()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("pasta legível").flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path
                    .file_name()
                    .is_some_and(|n| n != "platform" && n != "target")
                {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("arquivo legível");
                if text.contains(&format!("{}(target_os", "cfg")) {
                    offenders.push(path);
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "cfg de sistema fora de platform: {offenders:?}"
    );
}
