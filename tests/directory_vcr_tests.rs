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
async fn test_multiple_requests_directory_format() -> Result<(), Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new("multiple_requests_test")?;
    let vcr_client = setup.create_vcr_client().await?;

    // Make multiple different HTTP requests
    let urls = [
        "https://httpbin.org/get?test=1",
        "https://httpbin.org/get?test=2",
        "https://httpbin.org/post",
        "https://httpbin.org/put",
    ];

    let methods = [Method::Get, Method::Get, Method::Post, Method::Put];

    for (url, method) in urls.iter().zip(methods.iter()) {
        let mut request = http_types::Request::new(*method, Url::parse(url)?);

        // Add different body content for non-GET requests
        if *method != Method::Get {
            request.set_body(format!("test body for {method} request"));
        }

        let response = vcr_client.send(request).await?;
        println!("Request to {} returned status: {}", url, response.status());

        // Basic assertion that we got some response
        assert!(response.status().is_success() || response.status().is_informational());
    }

    Ok(())
}

#[tokio::test]
async fn test_repeated_requests_directory_format() -> Result<(), Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new("repeated_requests_test")?;
    let vcr_client = setup.create_vcr_client().await?;

    let url = "https://httpbin.org/json";

    // Make the same request multiple times to test replay consistency
    for i in 1..=3 {
        let request = http_types::Request::new(Method::Get, Url::parse(url)?);
        let response = vcr_client.send(request).await?;

        println!(
            "Request {} to {} returned status: {}",
            i,
            url,
            response.status()
        );
        assert!(response.status().is_success());

        // In replay mode, responses should be identical
        if matches!(setup.mode, VcrMode::Replay) {
            // We could add more specific assertions here if needed
            assert_eq!(response.status(), 200);
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_json_and_text_responses_directory_format() -> Result<(), Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new("json_text_responses_test")?;
    let vcr_client = setup.create_vcr_client().await?;

    // Test different content types
    let test_cases = [
        ("https://httpbin.org/json", "JSON response"),
        ("https://httpbin.org/html", "HTML response"),
        ("https://httpbin.org/xml", "XML response"),
    ];

    for (url, description) in test_cases {
        let request = http_types::Request::new(Method::Get, Url::parse(url)?);
        let response = vcr_client.send(request).await?;

        println!("{}: {}", description, response.status());
        assert!(response.status().is_success());

        // Verify we got some body content
        let mut response = response;
        let body = response.body_string().await?;
        assert!(
            !body.is_empty(),
            "Response body should not be empty for {url}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_sequential_identical_requests_directory_format() -> Result<(), Box<dyn std::error::Error>> {
    let setup = VcrTestSetup::new("sequential_identical_requests_test")?;
    let vcr_client = setup.create_vcr_client().await?;

    // Use httpbin.org/uuid which returns a different UUID each time
    let url = "https://httpbin.org/uuid";
    
    let mut responses = Vec::new();
    
    // Make the same request 3 times - each should get a different response in record mode
    for i in 1..=3 {
        let request = http_types::Request::new(Method::Get, Url::parse(url)?);
        let response = vcr_client.send(request).await?;
        
        println!("Request {} to {} returned status: {}", i, url, response.status());
        assert!(response.status().is_success());
        
        // Capture the response body to compare
        let mut response = response;
        let body = response.body_string().await?;
        responses.push(body);
        
        println!("Response {}: {}", i, responses[i-1]);
    }
    
    // In record mode, all responses should be different (different UUIDs)
    // In replay mode, we should get the same sequence of different responses
    if matches!(setup.mode, VcrMode::Record) {
        // In record mode, each UUID should be different
        assert_ne!(responses[0], responses[1], "First and second responses should be different");
        assert_ne!(responses[1], responses[2], "Second and third responses should be different");
        assert_ne!(responses[0], responses[2], "First and third responses should be different");
    } else {
        // In replay mode, we should get the same sequence as was recorded
        // The responses should still be different from each other (different UUIDs from recording)
        assert_ne!(responses[0], responses[1], "Replayed responses should maintain their differences");
        assert_ne!(responses[1], responses[2], "Replayed responses should maintain their differences");
        assert_ne!(responses[0], responses[2], "Replayed responses should maintain their differences");
    }

    Ok(())
}