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
async fn test_mixed_request_types_sequential_matching() -> Result<(), Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new("mixed_request_types_test")?;

    // Record a mix of different request types, some identical
    if matches!(setup.mode, VcrMode::Record) {
        let vcr_client = setup.create_vcr_client().await?;

        // Mix of different methods and URLs, with some duplicates
        let requests = [
            (Method::Get, "https://httpbin.org/uuid"),
            (Method::Post, "https://httpbin.org/post"),
            (Method::Get, "https://httpbin.org/uuid"), // duplicate
            (Method::Get, "https://httpbin.org/json"),
            (Method::Get, "https://httpbin.org/uuid"), // another duplicate
        ];

        for (i, (method, url)) in requests.iter().enumerate() {
            let mut request = http_types::Request::new(*method, Url::parse(url)?);
            if *method == Method::Post {
                request.set_body(format!("test body {i}"));
            }
            let response = vcr_client.send(request).await?;
            println!(
                "Recording {} request {} to {}: status {}",
                method,
                i + 1,
                url,
                response.status()
            );
        }
        return Ok(());
    }

    // In replay mode, verify that sequential matching works correctly with mixed request types
    let vcr_client = setup.create_vcr_client().await?;
    let mut responses = Vec::new();

    let requests = [
        (Method::Get, "https://httpbin.org/uuid"),
        (Method::Post, "https://httpbin.org/post"),
        (Method::Get, "https://httpbin.org/uuid"), // should get different response than first
        (Method::Get, "https://httpbin.org/json"),
        (Method::Get, "https://httpbin.org/uuid"), // should get different response than first two
    ];

    for (i, (method, url)) in requests.iter().enumerate() {
        let mut request = http_types::Request::new(*method, Url::parse(url)?);
        if *method == Method::Post {
            request.set_body(format!("test body {i}"));
        }
        let response = vcr_client.send(request).await?;
        let mut response = response;
        let body = response.body_string().await?;
        responses.push(body);
        println!("Replay {} request {} got: {}", method, i + 1, responses[i]);
    }

    // The UUID requests (indices 0, 2, 4) should have different responses
    assert_ne!(
        responses[0], responses[2],
        "Sequential identical UUID requests should get different responses"
    );
    assert_ne!(
        responses[0], responses[4],
        "Sequential identical UUID requests should get different responses"
    );
    assert_ne!(
        responses[2], responses[4],
        "Sequential identical UUID requests should get different responses"
    );

    println!("Mixed request types handled correctly with sequential matching");
    Ok(())
}
