//! Lazily started tokio runtime. russh needs one, the UI does not,
//! so nothing is spawned until the first connection.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

static RT: OnceLock<Runtime> = OnceLock::new();

pub fn rt() -> &'static Runtime {
    RT.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ssh-rt")
            .build()
            .expect("tokio runtime")
    })
}

/// Run `fut` on the tokio runtime, await the result from any executor (gpui's included).
pub async fn spawn<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> T {
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt().spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.await.expect("runtime task dropped")
}
