//! Warteschlange mit begrenzter Zahl gleichzeitiger Arbeiten: Aufträge werden der Reihe nach
//! abgearbeitet, höchstens `max` gleichzeitig (je ein Hintergrund-Thread). Läuft unabhängig von
//! der Oberfläche weiter (auch wenn das Fenster verkleinert ist). Nur Standardbibliothek.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

struct Stand<T> {
    liste: VecDeque<T>,
    /// Laufende Arbeiter-Threads.
    aktiv: usize,
}

type Arbeit<T> = Arc<dyn Fn(T) + Send + Sync>;

pub struct Warteschlange<T: Send + 'static> {
    stand: Arc<Mutex<Stand<T>>>,
    max: usize,
    arbeit: Arbeit<T>,
}

/// Sperre auch dann nehmen, wenn ein anderer Thread dabei abgestürzt ist (der Stand bleibt gültig:
/// er besteht nur aus der Liste und einem Zähler).
fn sperren<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl<T: Send + 'static> Warteschlange<T> {
    pub fn new(max: usize, arbeit: impl Fn(T) + Send + Sync + 'static) -> Self {
        Warteschlange {
            stand: Arc::new(Mutex::new(Stand { liste: VecDeque::new(), aktiv: 0 })),
            max: max.max(1),
            arbeit: Arc::new(arbeit),
        }
    }

    /// Höchstzahl gleichzeitiger Arbeiten.
    #[allow(dead_code)]
    pub fn max(&self) -> usize {
        self.max
    }

    /// Auftrag hinten anstellen; startet bei Bedarf einen weiteren Arbeiter.
    pub fn add(&self, item: T) {
        let mut g = sperren(&self.stand);
        g.liste.push_back(item);
        if g.aktiv >= self.max {
            return;
        }
        g.aktiv += 1;
        drop(g);
        let (stand, arbeit) = (self.stand.clone(), self.arbeit.clone());
        let gestartet = std::thread::Builder::new()
            .name("wandeln".into())
            .spawn(move || arbeiter(stand, arbeit))
            .is_ok();
        if !gestartet {
            // Kein Thread möglich: Auftrag bleibt in der Liste, der nächste `add` versucht es neu.
            let mut g = sperren(&self.stand);
            g.aktiv = g.aktiv.saturating_sub(1);
        }
    }

    /// Wie viele Aufträge noch warten (ohne die laufenden).
    #[allow(dead_code)]
    pub fn wartend(&self) -> usize {
        sperren(&self.stand).liste.len()
    }
}

fn arbeiter<T>(stand: Arc<Mutex<Stand<T>>>, arbeit: Arbeit<T>) {
    loop {
        let item = {
            let mut g = sperren(&stand);
            match g.liste.pop_front() {
                Some(x) => x,
                None => {
                    // Unter derselben Sperre abmelden wie `add` prüft: so bleibt nie ein
                    // Auftrag ohne Arbeiter liegen.
                    g.aktiv = g.aktiv.saturating_sub(1);
                    return;
                }
            }
        };
        let a = &arbeit;
        // Ein abgestürzter Auftrag darf die übrigen nicht aufhalten.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a(item)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn warten_bis(f: impl Fn() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(start.elapsed() < Duration::from_secs(20), "Zeitüberschreitung");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn hoechstens_max_gleichzeitig() {
        let jetzt = Arc::new(AtomicUsize::new(0));
        let spitze = Arc::new(AtomicUsize::new(0));
        let fertig = Arc::new(AtomicUsize::new(0));
        let (j, s, f) = (jetzt.clone(), spitze.clone(), fertig.clone());
        let q = Warteschlange::new(3, move |_: u32| {
            let n = j.fetch_add(1, Ordering::SeqCst) + 1;
            s.fetch_max(n, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(3));
            j.fetch_sub(1, Ordering::SeqCst);
            f.fetch_add(1, Ordering::SeqCst);
        });
        for i in 0..200 {
            q.add(i);
        }
        warten_bis(|| fertig.load(Ordering::SeqCst) == 200);
        assert!(spitze.load(Ordering::SeqCst) <= 3, "Spitze {}", spitze.load(Ordering::SeqCst));
        assert!(spitze.load(Ordering::SeqCst) >= 2);
        assert_eq!(q.wartend(), 0);
        // Nach dem Leerlauf nimmt sie wieder Aufträge an.
        q.add(1);
        warten_bis(|| fertig.load(Ordering::SeqCst) == 201);
    }

    #[test]
    fn absturz_haelt_nicht_auf() {
        let fertig = Arc::new(AtomicUsize::new(0));
        let f = fertig.clone();
        let q = Warteschlange::new(1, move |i: u32| {
            if i % 3 == 0 {
                panic!("Testabsturz {i}");
            }
            f.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(q.max(), 1);
        for i in 0..30 {
            q.add(i);
        }
        warten_bis(|| fertig.load(Ordering::SeqCst) == 20);
    }

    #[test]
    fn max_mindestens_eins() {
        let q = Warteschlange::new(0, |_: u8| {});
        assert_eq!(q.max(), 1);
    }
}
