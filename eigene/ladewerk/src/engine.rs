//! Warteschlange: ein Hintergrund-Thread verteilt die Downloads (so viele gleichzeitig wie
//! eingestellt), sammelt Fortschritt und Ergebnisse ein, wiederholt bei Fehlern, startet das
//! Entpacken fertiger Pakete und speichert den Zustand regelmässig. Er läuft auch dann weiter,
//! wenn das Fenster minimiert ist.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::net::{self, lock, Limiter, Outcome, Run};
use crate::plan;
use crate::sprache::t;
use crate::store::{self, Package, State, Status, Unpack};
use crate::unpack::{self, UnpackJob, UnpackRun};

/// Laufzeitdaten eines Links (werden nicht gespeichert).
#[derive(Default)]
pub struct Rt {
    pub run: Option<Arc<Run>>,
    /// Geglättete Geschwindigkeit in Bytes pro Sekunde.
    pub speed: f64,
    last: Option<(Instant, u64)>,
    pub retry_at: Option<Instant>,
    /// Auch starten, wenn die Warteschlange gestoppt ist (einzeln fortgesetzt).
    pub force: bool,
    pub remove: bool,
    pub delete_part: bool,
    pub reset: bool,
}

pub struct Model {
    pub state: State,
    pub rt: HashMap<u64, Rt>,
    pub unpack: HashMap<u64, Arc<UnpackRun>>,
    /// Pakete, die auf Wunsch jetzt entpackt werden sollen.
    pub unpack_now: HashSet<u64>,
    /// Warteschlange läuft (Start) oder ist gestoppt.
    pub active: bool,
    pub dirty: bool,
    pub save_error: String,
}

impl Model {
    pub fn rt(&mut self, id: u64) -> &mut Rt {
        self.rt.entry(id).or_default()
    }

    pub fn running(&self, id: u64) -> bool {
        self.rt.get(&id).is_some_and(|r| r.run.is_some())
    }

    pub fn speed(&self, id: u64) -> f64 {
        self.rt.get(&id).filter(|r| r.run.is_some()).map(|r| r.speed).unwrap_or(0.0)
    }

    pub fn total_speed(&self) -> f64 {
        self.rt.values().filter(|r| r.run.is_some()).map(|r| r.speed).sum()
    }

    pub fn active_count(&self) -> usize {
        self.rt.values().filter(|r| r.run.is_some()).count()
    }

    pub fn phase(&self, id: u64) -> Option<u8> {
        self.rt.get(&id).and_then(|r| r.run.as_ref()).map(|r| r.phase.load(Ordering::Relaxed))
    }

    fn cancel(&mut self, id: u64) {
        if let Some(run) = self.rt.get(&id).and_then(|r| r.run.as_ref()) {
            run.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn pause(&mut self, id: u64) {
        if let Some(it) = self.state.item_mut(id) {
            if matches!(it.status, Status::Waiting | Status::Running) {
                it.status = Status::Paused;
            }
        }
        self.rt(id).force = false;
        self.cancel(id);
        self.dirty = true;
    }

    /// Fortsetzen bzw. nach einem Fehler erneut versuchen.
    pub fn resume(&mut self, id: u64) {
        if let Some(it) = self.state.item_mut(id) {
            if matches!(it.status, Status::Paused | Status::Failed) {
                it.status = Status::Waiting;
                it.tries = 0;
                it.error.clear();
            }
        }
        let r = self.rt(id);
        r.retry_at = None;
        r.force = true;
        self.dirty = true;
    }

    pub fn pause_package(&mut self, pid: u64) {
        for id in self.item_ids(pid) {
            self.pause(id);
        }
    }

    pub fn resume_package(&mut self, pid: u64) {
        for id in self.item_ids(pid) {
            self.resume(id);
        }
    }

    pub fn item_ids(&self, pid: u64) -> Vec<u64> {
        self.state.packages.iter().find(|p| p.id == pid).map(|p| p.items.iter().map(|i| i.id).collect()).unwrap_or_default()
    }

    /// Von vorne laden (die eigene Teildatei wird verworfen, fertige Dateien bleiben).
    pub fn reset(&mut self, id: u64) {
        self.cancel(id);
        let r = self.rt(id);
        r.reset = true;
        r.force = true;
        self.dirty = true;
    }

    /// Aus der Liste nehmen. Fertige Dateien bleiben immer liegen.
    pub fn remove(&mut self, id: u64, delete_part: bool) {
        self.cancel(id);
        let r = self.rt(id);
        r.remove = true;
        r.delete_part = delete_part;
        self.dirty = true;
    }

    pub fn remove_package(&mut self, pid: u64, delete_parts: bool) {
        for id in self.item_ids(pid) {
            self.remove(id, delete_parts);
        }
        if let Some(u) = self.unpack.get(&pid) {
            u.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn remove_finished(&mut self) {
        let done: Vec<u64> = self.state.packages.iter().flat_map(|p| p.items.iter()).filter(|i| i.status == Status::Done).map(|i| i.id).collect();
        for id in done {
            self.remove(id, false);
        }
    }

    pub fn start_all(&mut self) {
        self.active = true;
    }

    /// Stoppt alle laufenden Downloads; sie warten danach wieder in der Schlange.
    pub fn stop_all(&mut self) {
        self.active = false;
        let ids: Vec<u64> = self.rt.iter().filter(|(_, r)| r.run.is_some()).map(|(id, _)| *id).collect();
        for id in ids {
            self.rt(id).force = false;
            self.cancel(id);
        }
    }

    pub fn add_package(&mut self, p: Package) {
        self.state.packages.push(p);
        self.dirty = true;
    }

    pub fn request_unpack(&mut self, pid: u64) {
        if let Some(p) = self.state.packages.iter_mut().find(|p| p.id == pid) {
            if p.unpack != Unpack::Running {
                p.unpack = Unpack::Idle;
                p.unpack_msg.clear();
                self.unpack_now.insert(pid);
                self.dirty = true;
            }
        }
    }
}

pub struct Engine {
    pub model: Arc<Mutex<Model>>,
    limiter: Arc<Limiter>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    path: Option<PathBuf>,
}

impl Engine {
    pub fn start(ctx: egui::Context) -> Engine {
        let path = store::state_file();
        let state = path.as_deref().map(store::load).unwrap_or_else(|| State::from_text(""));
        let active = state.settings.autostart;
        let model = Arc::new(Mutex::new(Model {
            state,
            rt: HashMap::new(),
            unpack: HashMap::new(),
            unpack_now: HashSet::new(),
            active,
            dirty: false,
            save_error: String::new(),
        }));
        let limiter = Arc::new(Limiter::default());
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (model, limiter, stop, path) = (model.clone(), limiter.clone(), stop.clone(), path.clone());
            std::thread::spawn(move || {
                let mut last_save = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    let (text, busy) = {
                        let mut g = lock(&model);
                        tick(&mut g, &limiter, &ctx, Instant::now());
                        let busy = g.rt.values().any(|r| r.run.is_some()) || !g.unpack.is_empty();
                        let text = if g.dirty && last_save.elapsed() >= Duration::from_secs(3) {
                            g.dirty = false;
                            Some(g.state.to_text())
                        } else {
                            None
                        };
                        (text, busy)
                    };
                    if let (Some(text), Some(p)) = (text, path.as_deref()) {
                        last_save = Instant::now();
                        let err = save_text(p, &text);
                        lock(&model).save_error = err;
                    }
                    if busy {
                        ctx.request_repaint();
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            })
        };
        Engine { model, limiter, stop, handle: Some(handle), path }
    }

    /// Beim Beenden: Verteiler anhalten, laufende Arbeit abbrechen, Zustand speichern.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let runs: Vec<Arc<Run>> = {
            let g = lock(&self.model);
            g.rt.values().filter_map(|r| r.run.clone()).collect()
        };
        let unpacks: Vec<Arc<UnpackRun>> = lock(&self.model).unpack.values().cloned().collect();
        for r in &runs {
            r.cancel.store(true, Ordering::Relaxed);
        }
        for u in &unpacks {
            u.cancel.store(true, Ordering::Relaxed);
        }
        // Kurz warten, damit Teildateien sauber geschlossen und 7-Zip beendet ist.
        for _ in 0..40 {
            if runs.iter().all(|r| r.finished.load(Ordering::Relaxed)) && unpacks.iter().all(|u| u.finished.load(Ordering::Relaxed)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let text = {
            let mut g = lock(&self.model);
            tick_collect(&mut g, Instant::now());
            g.state.to_text()
        };
        if let Some(p) = self.path.as_deref() {
            let _ = save_text(p, &text);
        }
        self.limiter.set_rate(0);
    }
}

fn save_text(path: &Path, text: &str) -> String {
    match store::save(path, text) {
        Ok(()) => String::new(),
        Err(e) => e.to_string(),
    }
}

/// Fortschritt und Ergebnisse der Download-Threads übernehmen.
fn tick_collect(m: &mut Model, now: Instant) {
    let settings = m.state.settings.clone();
    let Model { state, rt, dirty, .. } = m;
    for p in state.packages.iter_mut() {
        for it in p.items.iter_mut() {
            let Some(r) = rt.get_mut(&it.id) else { continue };
            let Some(run) = r.run.clone() else { continue };
            let done = run.done.load(Ordering::Relaxed);
            let total = run.total.load(Ordering::Relaxed);
            match r.last {
                Some((t0, d0)) => {
                    let dt = now.saturating_duration_since(t0).as_secs_f64();
                    if dt >= 0.2 {
                        let sample = done.saturating_sub(d0) as f64 / dt;
                        r.speed = plan::smooth(r.speed, sample, dt);
                        r.last = Some((now, done));
                    }
                }
                None => r.last = Some((now, done)),
            }
            it.done = done;
            if total != u64::MAX {
                it.size = Some(total);
            }
            {
                let meta = lock(&run.meta);
                if !meta.file_name.is_empty() && it.file_name != meta.file_name {
                    it.file_name = meta.file_name.clone();
                    *dirty = true;
                }
                if !meta.etag.is_empty() && it.etag != meta.etag {
                    it.etag = meta.etag.clone();
                }
                if !meta.last_modified.is_empty() && it.last_modified != meta.last_modified {
                    it.last_modified = meta.last_modified.clone();
                }
                if meta.sha_ok.is_some() {
                    it.sha_ok = meta.sha_ok;
                }
            }
            if !run.finished.load(Ordering::Relaxed) {
                continue;
            }
            let outcome = lock(&run.result).take().unwrap_or_else(|| Outcome::Retry(t("Download-Thread unerwartet beendet").to_string()));
            r.run = None;
            r.speed = 0.0;
            r.last = None;
            *dirty = true;
            match outcome {
                Outcome::Done => {
                    it.status = Status::Done;
                    it.error.clear();
                    it.tries = 0;
                    r.force = false;
                    if it.size.is_none() {
                        it.size = Some(it.done);
                    }
                }
                Outcome::Cancelled => {
                    if it.status == Status::Running {
                        it.status = Status::Waiting;
                    }
                }
                Outcome::Retry(msg) => {
                    if it.status == Status::Running {
                        it.tries = it.tries.saturating_add(1);
                        if it.tries <= settings.retries {
                            it.status = Status::Waiting;
                            r.retry_at = Some(now + Duration::from_secs(plan::retry_delay(settings.retry_delay_s, it.tries)));
                        } else {
                            it.status = Status::Failed;
                            r.force = false;
                        }
                    }
                    it.error = msg;
                }
                Outcome::Fatal(msg) => {
                    if it.status == Status::Running {
                        it.status = Status::Failed;
                        r.force = false;
                    }
                    it.error = msg;
                }
            }
        }
    }
}

fn tick(m: &mut Model, limiter: &Arc<Limiter>, ctx: &egui::Context, now: Instant) {
    limiter.set_rate(m.state.settings.speed_limit_kib.saturating_mul(1024));
    tick_collect(m, now);

    // Entfernen und Neustarten erst, wenn der Download-Thread fertig ist.
    let mut parts_to_delete: Vec<PathBuf> = Vec::new();
    {
        let Model { state, rt, dirty, .. } = m;
        for p in state.packages.iter_mut() {
            let dir = PathBuf::from(&p.dir);
            p.items.retain(|it| {
                let Some(r) = rt.get(&it.id) else { return true };
                if r.remove && r.run.is_none() {
                    if r.delete_part && it.status != Status::Done && !it.file_name.is_empty() {
                        parts_to_delete.push(net::part_path(&dir, &it.file_name));
                    }
                    *dirty = true;
                    return false;
                }
                true
            });
            for it in p.items.iter_mut() {
                let Some(r) = rt.get_mut(&it.id) else { continue };
                if r.reset && r.run.is_none() {
                    if it.status != Status::Done && !it.file_name.is_empty() {
                        parts_to_delete.push(net::part_path(&dir, &it.file_name));
                    }
                    it.file_name.clear();
                    it.done = 0;
                    it.size = None;
                    it.etag.clear();
                    it.last_modified.clear();
                    it.sha_ok = None;
                    it.tries = 0;
                    it.error.clear();
                    it.status = Status::Waiting;
                    r.reset = false;
                    r.retry_at = None;
                    *dirty = true;
                }
            }
        }
        let alive: HashSet<u64> = state.packages.iter().flat_map(|p| p.items.iter().map(|i| i.id)).collect();
        rt.retain(|id, r| alive.contains(id) || r.run.is_some());
        state.packages.retain(|p| !p.items.is_empty());
    }
    for p in parts_to_delete {
        let _ = std::fs::remove_file(p);
    }

    // Neue Downloads starten.
    let max = m.state.settings.max_parallel.clamp(1, 20) as usize;
    let mut running = m.active_count();
    let proxy = m.state.settings.proxy.clone();
    let active = m.active;
    let Model { state, rt, .. } = m;
    'outer: for p in state.packages.iter_mut() {
        for it in p.items.iter_mut() {
            if running >= max {
                break 'outer;
            }
            if it.status != Status::Waiting {
                continue;
            }
            let r = rt.entry(it.id).or_default();
            if r.run.is_some() || r.remove || r.reset || !(active || r.force) {
                continue;
            }
            if r.retry_at.is_some_and(|at| at > now) {
                continue;
            }
            let job = net::Job {
                url: it.url.clone(),
                dir: PathBuf::from(&p.dir),
                file_name: it.file_name.clone(),
                etag: it.etag.clone(),
                last_modified: it.last_modified.clone(),
                sha256: it.sha256.clone(),
                proxy: proxy.clone(),
            };
            let run = Arc::new(Run::new(it.done, it.size));
            it.status = Status::Running;
            it.sha_ok = None;
            r.run = Some(run.clone());
            r.retry_at = None;
            r.last = None;
            running += 1;
            let (limiter, ctx) = (limiter.clone(), ctx.clone());
            std::thread::spawn(move || {
                let outcome = net::download(&job, &run, &limiter);
                *lock(&run.result) = Some(outcome);
                run.finished.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            });
        }
    }

    // Entpacken.
    let settings = m.state.settings.clone();
    let Model { state, unpack, unpack_now, dirty, .. } = m;
    for p in state.packages.iter_mut() {
        if let Some(u) = unpack.get(&p.id).cloned() {
            let msg = lock(&u.status).clone();
            if !msg.is_empty() {
                p.unpack_msg = msg;
            }
            if u.finished.load(Ordering::Relaxed) {
                match lock(&u.result).take() {
                    Some(Ok(msg)) => {
                        p.unpack = Unpack::Done;
                        p.unpack_msg = msg;
                    }
                    Some(Err(msg)) => {
                        p.unpack = Unpack::Failed;
                        p.unpack_msg = msg;
                    }
                    None => {
                        p.unpack = Unpack::Failed;
                        p.unpack_msg = t("Entpacken unerwartet beendet").to_string();
                    }
                }
                unpack.remove(&p.id);
                *dirty = true;
            }
            continue;
        }
        let wanted = p.auto_extract || unpack_now.contains(&p.id);
        if p.unpack != Unpack::Idle || !wanted || p.items.is_empty() || !p.items.iter().all(|i| i.status == Status::Done) {
            continue;
        }
        unpack_now.remove(&p.id);
        let names: Vec<String> = p.items.iter().map(|i| i.file_name.clone()).filter(|n| !n.is_empty()).collect();
        if plan::archives_to_extract(&names).is_empty() {
            p.unpack = Unpack::Done;
            p.unpack_msg.clear();
            *dirty = true;
            continue;
        }
        let mut passwords = p.passwords.clone();
        passwords.extend(settings.passwords.iter().cloned());
        let job = UnpackJob {
            dir: PathBuf::from(&p.dir),
            names,
            passwords,
            sub_folder: settings.extract_subfolder,
            delete_archives: settings.delete_archives,
        };
        let u = Arc::new(UnpackRun::default());
        unpack.insert(p.id, u.clone());
        p.unpack = Unpack::Running;
        p.unpack_msg.clear();
        *dirty = true;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let res = unpack::run(&job, &u);
            *lock(&u.result) = Some(res);
            u.finished.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }
    // Erledigte Wünsche für nicht mehr vorhandene Pakete vergessen.
    let ids: HashSet<u64> = state.packages.iter().map(|p| p.id).collect();
    unpack_now.retain(|id| ids.contains(id));
    unpack.retain(|id, u| ids.contains(id) || !u.finished.load(Ordering::Relaxed));
}
