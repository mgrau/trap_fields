//! gmsh is a global, single-threaded library: one dedicated thread owns the [`App`] (and
//! with it gmsh) and executes commands sent from async handlers.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::mpsc;
use tf_app::App;

type Job = Box<dyn FnOnce(&mut App) + Send>;

#[derive(Clone)]
pub struct AppWorker {
    tx: mpsc::Sender<Job>,
}

impl AppWorker {
    pub fn spawn(dir: PathBuf) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new().name("app".into()).spawn(move || {
            let mut app = match App::new(&dir) {
                Ok(a) => {
                    let _ = ready_tx.send(Ok(()));
                    a
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            for job in rx {
                if catch_unwind(AssertUnwindSafe(|| job(&mut app))).is_err() {
                    tracing::error!("command panicked");
                }
            }
        })?;
        ready_rx.recv()??;
        Ok(Self { tx })
    }

    /// Run `f` on the app thread and await its result.
    pub async fn run<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut App) -> T + Send + 'static,
    {
        let (otx, orx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |app| {
                let _ = otx.send(f(app));
            }))
            .map_err(|_| anyhow::anyhow!("app worker has stopped"))?;
        orx.await.map_err(|_| anyhow::anyhow!("command panicked (see server log)"))
    }
}
