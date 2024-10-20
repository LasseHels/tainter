use crate::reconciler::{Condition, Configuration, Reconciler};
use crate::settings::Settings;
use actix_web::{get, App, HttpResponse, HttpServer, Responder};
use k8s_openapi::api::core::v1::Taint;
use kube::Client;
use regex::Regex;

pub struct Tainter {
    host: String,
    port: u16,
    reconciler: Reconciler,
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().body("healthy")
}

impl Tainter {
    pub fn new(settings: Settings, client: Client) -> Self {
        let matchers = Self::matchers(&settings);

        let reconciler = Reconciler::new(client, matchers);

        Tainter {
            host: settings.server.host,
            port: settings.server.port,
            reconciler,
        }
    }

    fn matchers(settings: &Settings) -> Vec<Configuration> {
        settings.reconciler.matchers.iter().map(|matcher| {
            let taint = Taint{
                effect: matcher.taint.effect.to_string(),
                key: matcher.taint.key.clone(),
                time_added: None,
                value: Some(matcher.taint.value.clone()),
            };

            let conditions: Vec<Condition> = matcher.conditions.iter().map(|cond| {
                Condition{
                    type_: Regex::new(cond.type_.as_str()).expect("regular expression should have been validated as part of initializing Settings"),
                    status: Regex::new(cond.status.as_str()).expect("regular expression should have been validated as part of initializing Settings"),
                }
            }).collect();

            Configuration{
                conditions,
                taint,
            }
        }).collect()
    }

    pub async fn start(self) -> std::io::Result<()> {
        tracing::info!("Starting Tainter");

        tokio::spawn(async move {
            tracing::info!("Starting reconciler");
            self.reconciler.start().await;
        });

        tracing::info!("Starting server");
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
