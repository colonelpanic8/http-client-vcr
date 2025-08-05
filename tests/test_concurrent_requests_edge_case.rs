use async_trait::async_trait;
use http_client::{Error, HttpClient, Request, Response};
use http_client_vcr::{CassetteFormat, DefaultMatcher, NoOpClient, VcrClient, VcrMode};
use http_types::{Method, Url};
use std::env;
use std::path::PathBuf;

// Simple adapter to make reqwest work with http-client trait
#[derive(Debug, Clone)]
struct ReqwestAdapter {
    client: reqwest::Client,
}

impl ReqwestAdapter {
    fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl HttpClient for ReqwestAdapter {
    async fn send(&self, mut req: Request) -> Result<Response, Error> {
        let method = match req.method() {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Delete => reqwest::Method::DELETE,
            Method::Head => reqwest::Method::HEAD,
            Method::Options => reqwest::Method::OPTIONS,
            Method::Patch => reqwest::Method::PATCH,
            _ => reqwest::Method::GET,
        };

        let mut reqwest_req = self.client.request(method, req.url().as_str());

        // Add headers
        for (name, values) in req.iter() {
            for value in values.iter() {
                reqwest_req = reqwest_req.header(name.as_str(), value.as_str());
            }
        }

        // Add body if present
        let body = req
            .body_string()
            .await
            .map_err(|e| Error::from_str(500, e))?;
        if !body.is_empty() {
            reqwest_req = reqwest_req.body(body);
        }

        let reqwest_resp = reqwest_req
            .send()
            .await
            .map_err(|e| Error::from_str(500, e))?;

        let mut response = Response::new(reqwest_resp.status().as_u16());

        // Copy headers
        for (name, value) in reqwest_resp.headers() {
            let _ = response.insert_header(name.as_str(), value.to_str().unwrap_or(""));
        }

        // Set body
        let body_bytes = reqwest_resp
            .bytes()
            .await
            .map_err(|e| Error::from_str(500, e))?;
        response.set_body(body_bytes.to_vec());

        Ok(response)
    }
}

/// Simple VCR test setup for directory-based cassettes
struct VcrTestSetup {
    cassette_path: PathBuf,
    mode: VcrMode,
}

impl VcrTestSetup {
    fn new(test_name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let cassette_path = PathBuf::from(format!("tests/fixtures/{test_name}"));

        // Check environment variable for recording mode
        let vcr_record = env::var("VCR_RECORD").unwrap_or_default();
        let cassette_exists = cassette_path.exists();

        let mode = match vcr_record.as_str() {
            "1" | "true" | "on" => VcrMode::Record,
            _ => {
                if !cassette_exists {
                    return Err(format!(
                        "No cassette found at '{}' and VCR_RECORD is not set. Either set VCR_RECORD=1 to record new interactions or ensure the cassette directory exists.",
                        cassette_path.display()
                    ).into());
                }
                VcrMode::Replay
            }
        };

        Ok(Self {
            cassette_path,
            mode,
        })
    }

    async fn create_vcr_client(&self) -> Result<VcrClient, Box<dyn std::error::Error>> {
        // Ensure test fixtures directory exists
        if let Some(parent_dir) = self.cassette_path.parent() {
            std::fs::create_dir_all(parent_dir)?;
        }

        let inner_client: Box<dyn HttpClient + Send + Sync> = match self.mode {
            VcrMode::Record => Box::new(ReqwestAdapter::new()),
            _ => Box::new(NoOpClient::new()),
        };

        // Create a custom matcher that only matches method and URL, ignoring headers
        let matcher = DefaultMatcher::new().with_headers(vec![]); // Don't match any headers

        let vcr_client = VcrClient::builder(&self.cassette_path)
            .inner_client(inner_client)
            .mode(self.mode.clone())
            .format(CassetteFormat::Directory) // Use directory format
            .matcher(Box::new(matcher))
            .build()
            .await?;

        Ok(vcr_client)
    }
}

#[tokio::test]
async fn test_concurrent_requests_edge_case() -> Result<(), Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new("concurrent_requests_test")?;

    // First, record multiple different UUID requests
    if matches!(setup.mode, VcrMode::Record) {
        let vcr_client = setup.create_vcr_client().await?;
        let url = "https://httpbin.org/uuid";

        for i in 1..=3 {
            let request = http_types::Request::new(Method::Get, Url::parse(url)?);
            let response = vcr_client.send(request).await?;
            println!("Recording request {}: status {}", i, response.status());
        }
        return Ok(());
    }

    // In replay mode, test concurrent access to the same VCR client
    let vcr_client = std::sync::Arc::new(setup.create_vcr_client().await?);
    let url = "https://httpbin.org/uuid";

    // Create multiple concurrent tasks that make identical requests
    let mut handles = Vec::new();
    for i in 1..=3 {
        let client = vcr_client.clone();
        let url = url.to_string();
        let handle = tokio::spawn(async move {
            let request = http_types::Request::new(Method::Get, Url::parse(&url)?);
            let response = client.send(request).await?;
            let mut response = response;
            let body = response.body_string().await?;
            println!("Concurrent request {i} got: {body}");
            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(body)
        });
        handles.push(handle);
    }

    // Wait for all requests to complete
    let mut responses = Vec::new();
    for handle in handles {
        let response = handle.await.unwrap().unwrap();
        responses.push(response);
    }

    // All responses should be different (since we recorded different UUIDs)
    // But with concurrent access, we might see race conditions in the used_interactions tracking
    assert_ne!(
        responses[0], responses[1],
        "Concurrent requests should get different responses"
    );
    assert_ne!(
        responses[1], responses[2],
        "Concurrent requests should get different responses"
    );
    assert_ne!(
        responses[0], responses[2],
        "Concurrent requests should get different responses"
    );

    println!("Concurrent requests handled correctly: {responses:?}");
    Ok(())
}
