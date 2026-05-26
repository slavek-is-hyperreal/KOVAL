use crate::config::Config;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use schema::{TokenRequest, TokenResponse, TokenRecord, JobSummary, WebhookRequest, WebhookRecord};
use serde::de::DeserializeOwned;
use std::time::Duration;

pub struct ApiClient {
    client: Client,
    server_url: String,
}

impl ApiClient {
    pub fn new(config: &Config) -> Result<Self, String> {
        let server_url = config.server_url.clone().ok_or(
            "Server URL is not set. Run: koval config set-server <url>".to_string()
        )?;
        let token = config.token.clone().ok_or(
            "Bearer token is not set. Run: koval config set-token <token>".to_string()
        )?;

        let mut headers = HeaderMap::new();
        
        let auth_val = format!("Bearer {}", token);
        let mut auth_header = HeaderValue::from_str(&auth_val)
            .map_err(|e| format!("Invalid authorization header value: {}", e))?;
        auth_header.set_sensitive(true);
        
        headers.insert(AUTHORIZATION, auth_header);

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("Failed to build reqwest client: {}", e))?;

        // Ensure server_url ends without slash
        let clean_url = server_url.trim_end_matches('/').to_string();

        Ok(Self {
            client,
            server_url: clean_url,
        })
    }

    fn post<T: serde::Serialize, R: DeserializeOwned>(&self, path: &str, body: &T) -> Result<R, String> {
        let url = format!("{}{}", self.server_url, path);
        let resp = self.client.post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .map_err(|e| format!("Network request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Server returned error ({}): {}", status, text));
        }

        resp.json::<R>().map_err(|e| format!("Failed to parse response JSON: {}", e))
    }

    fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R, String> {
        let url = format!("{}{}", self.server_url, path);
        let resp = self.client.get(&url)
            .send()
            .map_err(|e| format!("Network request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Server returned error ({}): {}", status, text));
        }

        resp.json::<R>().map_err(|e| format!("Failed to parse response JSON: {}", e))
    }

    fn delete(&self, path: &str) -> Result<(), String> {
        let url = format!("{}{}", self.server_url, path);
        let resp = self.client.delete(&url)
            .send()
            .map_err(|e| format!("Network request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Server returned error ({}): {}", status, text));
        }

        Ok(())
    }

    // --- Web API Call Wrappers ---

    pub fn create_token(&self, name: &str) -> Result<TokenResponse, String> {
        let req = TokenRequest { name: name.to_string() };
        self.post("/tokens", &req)
    }

    pub fn list_tokens(&self) -> Result<Vec<TokenRecord>, String> {
        self.get("/tokens")
    }

    pub fn delete_token(&self, id: i64) -> Result<(), String> {
        self.delete(&format!("/tokens/{}", id))
    }

    pub fn list_jobs(&self) -> Result<Vec<JobSummary>, String> {
        self.get("/jobs")
    }

    pub fn job_status(&self, job_id: &str) -> Result<serde_json::Value, String> {
        self.get(&format!("/build/{}/status", job_id))
    }

    pub fn create_webhook(&self, url: &str, secret: &str) -> Result<serde_json::Value, String> {
        let req = WebhookRequest {
            url: url.to_string(),
            secret: secret.to_string(),
        };
        self.post("/webhooks", &req)
    }

    pub fn list_webhooks(&self) -> Result<Vec<WebhookRecord>, String> {
        self.get("/webhooks")
    }

    pub fn delete_webhook(&self, id: i64) -> Result<(), String> {
        self.delete(&format!("/webhooks/{}", id))
    }

    pub fn submit_job(&self, req: &schema::JobRequest) -> Result<serde_json::Value, String> {
        self.post("/build", req)
    }

    pub fn upload_pgo_profiles(&self, instrument_job_id: &str, files: Vec<(String, Vec<u8>)>) -> Result<schema::PgoUploadResponse, String> {
        let url = format!("{}/pgo/profiles/{}", self.server_url, instrument_job_id);
        let mut form = reqwest::blocking::multipart::Form::new();
        
        for (filename, content) in files {
            let part = reqwest::blocking::multipart::Part::bytes(content)
                .file_name(filename);
            form = form.part("profile", part);
        }

        let resp = self.client.post(&url)
            .multipart(form)
            .send()
            .map_err(|e| format!("Network request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("Server returned error ({}): {}", status, text));
        }

        resp.json::<schema::PgoUploadResponse>().map_err(|e| format!("Failed to parse response JSON: {}", e))
    }
}
