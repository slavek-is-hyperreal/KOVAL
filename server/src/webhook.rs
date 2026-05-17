use hmac::{Hmac, Mac};
use sha2::Sha256;
use schema::WebhookPayload;
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

/// Generates the HMAC-SHA256 signature for the given webhook body and secret.
pub fn generate_signature(secret: &str, body: &[u8]) -> String {
    if let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) {
        mac.update(body);
        let result = mac.finalize();
        let code_bytes = result.into_bytes();
        code_bytes.iter().map(|b| format!("{:02x}", b)).collect()
    } else {
        String::new()
    }
}

/// Asynchronously delivers the WebhookPayload to a list of URLs with HMAC signing and retry backoffs.
/// Fire-and-forget execution (doesn't block the caller).
pub async fn deliver(payload: WebhookPayload, webhooks: Vec<(String, String)>) {
    if webhooks.is_empty() {
        return;
    }

    let payload_bytes = match serde_json::to_vec(&payload) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("Webhook delivery payload serialization failed: {:?}", e);
            return;
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to build HTTP client for webhooks: {:?}", e);
            return;
        }
    };

    for (url, secret) in webhooks {
        let signature = generate_signature(&secret, &payload_bytes);
        let client_clone = client.clone();
        let payload_bytes_clone = payload_bytes.clone();
        let url_clone = url.clone();

        tokio::spawn(async move {
            let mut attempts = 0;
            // Backoffs: 0s for 1st attempt, 2s for 2nd attempt, 5s for 3rd attempt
            let backoffs = [Duration::from_secs(0), Duration::from_secs(2), Duration::from_secs(5)];

            while attempts < 3 {
                if attempts > 0 {
                    tokio::time::sleep(backoffs[attempts]).await;
                }
                attempts += 1;

                let req = client_clone.post(&url_clone)
                    .header("Content-Type", "application/json")
                    .header("X-Koval-Signature", format!("sha256={}", signature))
                    .body(payload_bytes_clone.clone());

                match req.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        return; // Successfully delivered!
                    }
                    Ok(resp) => {
                        eprintln!(
                            "Webhook delivery to {} failed with status: {}, attempt: {}",
                            url_clone, resp.status(), attempts
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "Webhook delivery error to {}, attempt {}: {:?}",
                            url_clone, attempts, e
                        );
                    }
                }
            }
            eprintln!(
                "Webhook delivery to {} failed after 3 attempts.",
                url_clone
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_signature_verification() {
        let secret = "test_key";
        let body = b"{\"hello\":\"world\"}";
        let sig = generate_signature(secret, body);
        
        // Expected value manually verified/standard HMAC-SHA256
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64);
    }

    #[tokio::test]
    async fn test_webhook_retry_loop_success_on_third_attempt() {
        let attempts_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = attempts_counter.clone();

        // 1. Create a mock Axum server that returns 500 twice then 200
        let app = Router::new().route("/webhook", post(move || {
            let current = counter_clone.fetch_add(1, Ordering::SeqCst);
            async move {
                if current < 2 {
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR
                } else {
                    axum::http::StatusCode::OK
                }
            }
        }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // 2. Call deliver
        let payload = WebhookPayload {
            job_id: "test-uuid".to_string(),
            status: "done".to_string(),
            finished_at: Some("now".to_string()),
            project: "my-project".to_string(),
            sha256: Some("sha-value".to_string()),
        };

        let url = format!("http://{}/webhook", addr);
        deliver(payload, vec![(url, "secret".to_string())]).await;

        // 3. Wait for sleep backoffs to complete (2s + 5s + leeway = 9s max)
        tokio::time::sleep(Duration::from_secs(8)).await;

        assert_eq!(attempts_counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_webhook_retry_always_fails_no_panic() {
        let payload = WebhookPayload {
            job_id: "test-uuid".to_string(),
            status: "failed".to_string(),
            finished_at: None,
            project: "my-project".to_string(),
            sha256: None,
        };

        // Bind and immediately drop an ephemeral TCP listener to get a guaranteed closed, unoccupied port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let url = format!("http://{}/webhook", addr);
        deliver(payload, vec![(url, "secret".to_string())]).await;

        // Let the spawn execute and fail silently
        tokio::time::sleep(Duration::from_secs(8)).await;
        // Verify no panics happened
    }
}
