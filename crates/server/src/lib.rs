pub mod app;
pub mod config;
pub mod crypto;
pub mod executor;
pub mod flows;
pub mod icons;
pub mod state;
pub mod store;

use std::sync::Arc;

/// Levanta el servidor en 127.0.0.1 (puerto 0 = uno libre). Devuelve el puerto real.
pub async fn serve(core: Arc<state::Core>) -> std::io::Result<(u16, tokio::task::JoinHandle<()>)> {
    let port = core.cfg.read().unwrap().port;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let port = listener.local_addr()?.port();
    core.cfg.write().unwrap().port = port;
    let app = app::router(state::Hub(Arc::new(std::sync::RwLock::new(core))));
    let h = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Ok((port, h))
}
