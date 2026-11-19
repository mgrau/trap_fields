//! gmsh is a global, single-threaded library: one dedicated thread owns it (and the
//! loaded OCC model) and executes jobs sent from async handlers.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use tf_geom::Model;
use tf_gmsh::Gmsh;

pub struct Ctx {
    pub gmsh: Gmsh,
    pub model: Option<Model>,
}

type Job = Box<dyn FnOnce(&mut Ctx) + Send>;

#[derive(Clone)]
pub struct GmshWorker {
    tx: mpsc::Sender<Job>,
}

impl GmshWorker {
    pub fn spawn() -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new().name("gmsh".into()).spawn(move || {
            let gmsh = match Gmsh::initialize() {
                Ok(g) => {
                    let _ = ready_tx.send(Ok(()));
                    g
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let mut ctx = Ctx { gmsh, model: None };
            for job in rx {
                if catch_unwind(AssertUnwindSafe(|| job(&mut ctx))).is_err() {
                    tracing::error!("gmsh job panicked");
                }
            }
        })?;
        ready_rx.recv()??;
        Ok(Self { tx })
    }

    /// Run `f` on the gmsh thread and await its result.
    pub async fn run<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Ctx) -> T + Send + 'static,
    {
        let (otx, orx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |ctx| {
                let _ = otx.send(f(ctx));
            }))
            .map_err(|_| anyhow::anyhow!("gmsh worker has stopped"))?;
        orx.await.map_err(|_| anyhow::anyhow!("gmsh job panicked"))
    }
}
