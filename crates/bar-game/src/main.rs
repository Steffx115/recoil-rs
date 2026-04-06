use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Recoil RTS engine starting...");

    // TODO: Initialize sim world, open window, start game loop
    tracing::info!("No game loop yet — Sprint 0 scaffold only.");
}
