// Unter Windows ohne Konsolenfenster starten.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assoc;
mod core;
mod gui;
mod icons;
mod meldung;
mod sprache;
mod feeds;
mod weather;

use crate::core::{fmt_version, App, Kind, Manifest, Version};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    core::launcher_startup();
    core::init_token();
    gui::init_theme();
    // Erhöhte Instanz (siehe core::create_dir_elevated): nur den Ordner anlegen, dann beenden.
    if args.first().map(String::as_str) == Some("--mkdir-elevated") {
        let (Some(dir), Some(user)) = (args.get(1), args.get(2)) else { std::process::exit(2) };
        if let Err(e) = core::mkdir_elevated(std::path::Path::new(dir), user) {
            eprintln!("Fehler: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    // Von Windows aufgerufen («Öffnen mit»): Datei im zugeordneten Programm öffnen.
    if args.first().map(String::as_str) == Some("--open") {
        let (Some(id), Some(file)) = (args.get(1), args.get(2)) else { std::process::exit(2) };
        let manifest = core::load_manifest().unwrap_or_else(|_| Manifest { title: "Werkbank".into(), app: Vec::new(), spenden: None, fremd: Vec::new() });
        if let Some(app) = manifest.app.iter().find(|a| &a.id == id) {
            if core::run_with(app, Some(std::path::Path::new(file))).is_ok() {
                return;
            }
            let name = app.name.clone();
            gui::run(move || manifest.clone(), None, true, Some(sprache::tf("{name} ist nicht installiert. Zum Öffnen von {datei} zuerst installieren.", &[("name", &name), ("datei", file)])));
            return;
        }
        std::process::exit(1);
    }
    let load = || match core::load_manifest() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Fehler: {e:#}");
            std::process::exit(1);
        }
    };
    // Offline-Setup: ein Ordner `bundle` neben dem Launcher (oder --bundle DIR) wird beim
    // Start eingerichtet; --offline verzichtet auf die Suche nach neueren Versionen.
    let bundle = args
        .iter()
        .position(|a| a == "--bundle")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let dir = std::env::current_exe().ok()?.parent()?.join("bundle");
            dir.is_dir().then_some(dir)
        });
    let online = !args.iter().any(|a| a == "--offline");
    // Übrige Argumente ohne Optionen: leer → Oberfläche, sonst Kommandozeile.
    let mut plain = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--bundle" {
            it.next();
        } else if !a.starts_with("--") {
            plain.push(a.clone());
        }
    }
    if plain.is_empty() {
        gui::run(load, bundle, online, None);
    } else if let Err(e) = cli(&load(), &args) {
        eprintln!("Fehler: {e:#}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------------------------
// Kommandozeile

fn cli(manifest: &Manifest, args: &[String]) -> anyhow::Result<()> {
    let find = |id: &str| {
        manifest
            .app
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| anyhow::anyhow!("unbekanntes Programm: {id}"))
    };
    let targets = |id: Option<&String>| -> anyhow::Result<Vec<&App>> {
        match id.map(String::as_str) {
            None | Some("all") | Some("alle") => Ok(manifest.app.iter().collect()),
            Some(id) => Ok(vec![find(id)?]),
        }
    };
    let quiet = |_: &str, _: f32| {};
    let show = |v: Option<Version>| v.map(fmt_version).unwrap_or_else(|| "–".into());
    match args[0].as_str() {
        "list" => {
            println!("{:<13} {:<11} {:<11} {}", "Programm", "Desktop", "Browser", "Neueste");
            for app in &manifest.app {
                if let Some(f) = &app.fremd {
                    let state = match (core::fremd_find(f).is_some(), core::fremd_version(f)) {
                        (true, Some(v)) => fmt_version(v),
                        (true, None) => "installiert".into(),
                        _ => "–".into(),
                    };
                    let latest = match &f.github {
                        Some(_) => core::fremd_latest(f).map(fmt_version).unwrap_or_else(|e| format!("? ({e:#})")),
                        None => "Fremdprogramm, aktualisiert sich selbst".into(),
                    };
                    println!("{:<13} {:<11} {:<11} {}", app.id, state, "–", latest);
                    continue;
                }
                let latest = core::latest_version(app).map(fmt_version).unwrap_or_else(|e| format!("? ({e:#})"));
                println!(
                    "{:<13} {:<11} {:<11} {}",
                    app.id,
                    show(core::installed_version(app, Kind::Native)),
                    show(core::installed_version(app, Kind::Web)),
                    latest
                );
            }
        }
        "install" | "update" => {
            let kind = if args.iter().any(|a| a == "--web") { Kind::Web } else { Kind::Native };
            let id = args.iter().skip(1).find(|a| !a.starts_with("--"));
            for app in targets(id)? {
                // Fremdprogramme: nur auf ausdrücklichen Wunsch («install brave»), nie aktualisieren.
                if let Some(f) = &app.fremd {
                    if args[0] == "install" && id.is_some_and(|i| i != "all" && i != "alle") {
                        core::fremd_install(f, &quiet)?;
                        println!("{}: installiert (aktualisiert sich selbst)", app.id);
                    }
                    continue;
                }
                // "update" ohne Namen fasst nur an, was schon installiert ist.
                if args[0] == "update" && id.is_none() && core::installed_version(app, kind).is_none() {
                    continue;
                }
                let before = core::installed_version(app, kind);
                let v = core::install(app, kind, &quiet)?;
                match before {
                    Some(b) if b == v => println!("{}: {} ist aktuell", app.id, fmt_version(v)),
                    Some(b) => println!("{}: {} → {}", app.id, fmt_version(b), fmt_version(v)),
                    None => println!("{}: {} installiert", app.id, fmt_version(v)),
                }
            }
        }
        "install-bundle" => {
            let dir = args.iter().skip(1).find(|a| !a.starts_with("--")).ok_or_else(|| anyhow::anyhow!("Aufruf: install-bundle <ordner> [--offline]"))?;
            let online = !args.iter().any(|a| a == "--offline");
            core::install_bundle(
                manifest,
                std::path::Path::new(dir),
                online,
                &|_, _, _| {},
                &|app, kind, r| match r {
                    Ok(v) => println!("{}: {} eingerichtet ({kind:?})", app.id, fmt_version(v)),
                    Err(e) => println!("{}: FEHLER {e:#}", app.id),
                },
            );
        }
        "install-file" => {
            let (id, file) = (args.get(1), args.get(2));
            let (Some(id), Some(file)) = (id, file) else { anyhow::bail!("Aufruf: install-file <programm> <archiv>") };
            let (kind, v) = core::install_file(find(id)?, std::path::Path::new(file), &quiet)?;
            println!("{id}: {} aus Datei installiert ({kind:?})", fmt_version(v));
        }
        "remove" => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("Aufruf: remove <programm>"))?;
            core::uninstall(find(id)?)?;
            println!("{id}: entfernt");
        }
        "run" => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("Aufruf: run <programm>"))?;
            core::run(find(id)?)?;
        }
        "web" => {
            let addr = core::serve_web(manifest.clone())?;
            let url = match args.iter().skip(1).find(|a| !a.starts_with("--")) {
                Some(id) => {
                    let app = find(id)?;
                    core::install(app, Kind::Web, &quiet)?;
                    format!("{addr}/{id}/")
                }
                None => addr,
            };
            println!("Browser-Versionen laufen unter {url}  (Beenden mit Strg+C)");
            if !args.iter().any(|a| a == "--no-open") {
                let _ = open::that(&url);
            }
            loop {
                std::thread::park();
            }
        }
        "dir" => println!("{}", core::data_dir().display()),
        _ => {
            println!(
                "Aufruf ohne Argumente öffnet die Oberfläche.\n\n\
                 list                          Programme und Versionen anzeigen\n\
                 install <programm|all> [--web]  installieren\n\
                 update [programm] [--web]     aktualisieren (ohne Namen: alle installierten)\n\
                 install-file <programm> <archiv>  selbst gebautes Archiv installieren
                 install-bundle <ordner> [--offline]  Archive eines Offline-Setups einrichten\n\
                 run <programm>                starten\n\
                 web [programm]                Browser-Version(en) lokal bereitstellen\n\
                 remove <programm>             entfernen\n\
                 dir                           Datenordner anzeigen"
            );
        }
    }
    Ok(())
}
