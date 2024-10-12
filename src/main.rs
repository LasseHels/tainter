use k8s_openapi::serde::Deserialize;

mod reconciler;
mod settings;
mod tainter;

#[actix_web::main]
async fn main() {
    // TODO read settings file and initialize.
    // let settings = Config::builder().add_source(config::File::with_name("example_config")).build().unwrap();
    //
    // println!("{:?}", settings);

    tracing_subscriber::fmt()
        .json()
        // TODO ability to configure log level.
        .with_max_level(tracing::Level::INFO)
        .with_current_span(false)
        .init();

    // let tainter = tainter::Tainter::new("localhost".to_string(), 8000);
    // tainter.start().await.unwrap();
}
