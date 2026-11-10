use crate::config::{ConfigStore, ModelService};
use reqwest::blocking::Client;
use reqwest::{redirect::Policy, Url};
use serde::Deserialize;
use std::io::Read;
use std::time::Duration;

#[tauri::command]
pub(crate) async fn list_service_models(service_id: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = ConfigStore::load().map_err(|_| "Cannot load model services")?;
        let service = config
            .model_services
            .iter()
            .find(|service| service.id == service_id)
            .ok_or("Model service is no longer available")?;
        fetch_models(service)
    })
    .await
    .map_err(|_| "Model list request interrupted".to_string())?
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
    #[serde(default)]
    domain: String,
    modalities: Option<Modalities>,
}

#[derive(Deserialize)]
struct Modalities {
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    output_modalities: Vec<String>,
}

impl ListedModel {
    fn ark_text_model(&self) -> bool {
        let accepts_text = |values: &[String]| {
            values.is_empty() || values.iter().any(|v| v.eq_ignore_ascii_case("text"))
        };
        if let Some(modes) = &self.modalities {
            if !modes.output_modalities.is_empty() {
                return accepts_text(&modes.input_modalities)
                    && accepts_text(&modes.output_modalities);
            }
        }
        matches!(self.domain.as_str(), "" | "LLM" | "VLM")
    }
}

fn catalog_url(endpoint: &str) -> Result<Url, String> {
    let mut url = Url::parse(endpoint.trim()).map_err(|_| "Invalid service URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Invalid service URL".into());
    }
    let path = url.path().trim_end_matches('/');
    let base = ["/chat/completions", "/responses", "/models"]
        .iter()
        .find_map(|suffix| path.strip_suffix(suffix))
        .unwrap_or(path);
    url.set_path(&format!("{base}/models"));
    url.set_fragment(None);
    Ok(url)
}

fn fetch_models(service: &ModelService) -> Result<Vec<String>, String> {
    if service.api_key.trim().is_empty() {
        return Err("API key required".into());
    }
    let url = catalog_url(&service.endpoint)?;
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot initialize model list request")?;
    let response = client
        .get(url)
        .bearer_auth(service.api_key.trim())
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                "Request timed out"
            } else {
                "Connection failed"
            }
        })?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read model list")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Model list is too large".into());
    }
    let list: ModelList = serde_json::from_slice(&bytes).map_err(|_| "Invalid model list")?;
    let mut ids: Vec<_> = list
        .data
        .into_iter()
        .filter(|model| service.provider != "volc_ark" || model.ark_text_model())
        .map(|model| model.id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    #[test]
    fn uses_saved_base_path_and_bearer_auth_and_keeps_ark_text_models() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut service = crate::config::AppConfig::default().model_services[1].clone();
        service.endpoint = format!("http://{}/api/v3/responses", listener.local_addr().unwrap());
        service.api_key = "fixture-key".into();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            let mut byte = [0; 1];
            while !data.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                data.push(byte[0]);
            }
            let headers = String::from_utf8(data).unwrap().to_lowercase();
            assert!(headers.starts_with("get /api/v3/models http/1.1"));
            assert!(headers.contains("authorization: bearer fixture-key\r\n"));
            let body = r#"{"data":[{"id":"vision-text","domain":"VLM"},{"id":"only-images","modalities":{"output_modalities":["image"]}},{"id":"new-model"},{"id":"new-model"}]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        assert_eq!(
            fetch_models(&service).unwrap(),
            ["new-model", "vision-text"]
        );
        server.join().unwrap();
        assert_eq!(
            catalog_url("https://api.openai.com/v1/").unwrap().as_str(),
            "https://api.openai.com/v1/models"
        );
    }
}
