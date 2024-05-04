mod tainter;

#[actix_web::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        // TODO ability to configure log level.
        .with_max_level(tracing::Level::INFO)
        .with_current_span(false)
        .init();

    let tainter = tainter::Tainter::new("localhost".to_string(), 8000);
    tainter.start().await.unwrap();
}
