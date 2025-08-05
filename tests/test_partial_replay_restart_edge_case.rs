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
async fn test_partial_replay_restart_edge_case() -> Result<(), Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new("partial_replay_restart_test")?;

    // First, let's record 4 requests to httpbin.org/uuid
    if matches!(setup.mode, VcrMode::Record) {
        let vcr_client = setup.create_vcr_client().await?;
        let url = "https://httpbin.org/uuid";

        for i in 1..=4 {
            let request = http_types::Request::new(Method::Get, Url::parse(url)?);
            let response = vcr_client.send(request).await?;
            println!("Recording request {}: status {}", i, response.status());
        }
        return Ok(());
    }

    // In replay mode, test the edge case
    let url = "https://httpbin.org/uuid";
    let mut first_client_responses = Vec::new();

    // First VCR client - consume first 2 interactions
    {
        let vcr_client = setup.create_vcr_client().await?;
        for i in 1..=2 {
            let request = http_types::Request::new(Method::Get, Url::parse(url)?);
            let response = vcr_client.send(request).await?;
            let mut response = response;
            let body = response.body_string().await?;
            first_client_responses.push(body);
            println!(
                "First client request {}: {}",
                i,
                first_client_responses[i - 1]
            );
        }
        // vcr_client goes out of scope here, used_interactions is lost
    }

    // Second VCR client - should start fresh and get the SAME first 2 interactions again
    // This demonstrates the potential issue: partial consumption state is lost
    let mut second_client_responses = Vec::new();
    {
        let vcr_client = setup.create_vcr_client().await?;
        for i in 1..=2 {
            let request = http_types::Request::new(Method::Get, Url::parse(url)?);
            let response = vcr_client.send(request).await?;
            let mut response = response;
            let body = response.body_string().await?;
            second_client_responses.push(body);
            println!(
                "Second client request {}: {}",
                i,
                second_client_responses[i - 1]
            );
        }
    }

    // The issue: second client gets the same responses as first client
    // instead of continuing from where first client left off
    println!("First client got: {first_client_responses:?}");
    println!("Second client got: {second_client_responses:?}");

    // This assertion will PASS, demonstrating the bug
    assert_eq!(
        first_client_responses[0], second_client_responses[0],
        "Second client should get same first response - demonstrating the restart bug"
    );
    assert_eq!(
        first_client_responses[1], second_client_responses[1],
        "Second client should get same second response - demonstrating the restart bug"
    );

    println!("BUG DEMONSTRATED: Both clients got identical sequences instead of continuing from where the previous client left off");

    Ok(())
}
