use actix_web::{get, App, HttpResponse, HttpServer, Responder};

pub struct Tainter {
    host: String,
    port: u16,
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().body("healthy")
}

impl Tainter {
    pub fn new(host: String, port: u16) -> Self {
        Tainter { host, port }
    }

    pub async fn start(&self) -> std::io::Result<()> {
        tracing::info!("Starting Tainter");

        HttpServer::new(|| App::new().service(health))
            .bind((self.host.as_str(), self.port))?
            .run()
            .await
    }
}

#[cfg(test)]
mod tests {
    use actix_web::{test, App};

    use super::*;

    #[actix_web::test]
    async fn test_health_endpoint() {
        let app = test::init_service(App::new().service(health)).await;

        let req = test::TestRequest::default().uri("/health").to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
        let body = test::read_body(resp).await;
        assert_eq!(body, actix_web::web::Bytes::from("healthy"));
    }
}
