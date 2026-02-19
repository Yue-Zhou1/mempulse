use anyhow::Result;
use tokio::net::TcpListener;
use viz_api::{build_router, default_state};

#[tokio::main]
async fn main() -> Result<()> {
    let app = build_router(default_state());
    let listener = TcpListener::bind("127.0.0.1:3000").await?;
    println!("viz-api listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await?;
    Ok(())
}
