# HTTP Client VCR

A Rust library for recording and replaying HTTP requests, inspired by VCR libraries in other languages. This library works with the `http-client` crate to provide a simple way to test HTTP interactions.

## Features

- **Record HTTP interactions** to YAML cassettes
- **Replay recorded interactions** for deterministic tests
- **Multiple recording modes** (Record, Replay, Once, None)
- **Flexible request matching** (URL, method, headers, body)
- **Builder pattern** for easy configuration
- **Thread-safe** with async support

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
http-client-vcr = "0.1.0"
```

### Basic Example

```rust
use http_client_vcr::{VcrClient, VcrMode};
use http_client::Request;
use http_types::{Method, Url};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create your HTTP client (h1, hyper, isahc, etc.)
    let inner_client = Box::new(h1::H1Client::new());
    
    // Create VCR client
    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .cassette_path("fixtures/my_test.yaml")
        .mode(VcrMode::Once)
        .build()
        .await?;
    
    // Use it like any HttpClient
    let request = Request::new(Method::Get, Url::parse("https://httpbin.org/get")?);
    let response = vcr_client.send(request).await?;
    
    println!("Status: {}", response.status());
    
    // Save cassette
    vcr_client.save_cassette().await?;
    
    Ok(())
}
```

## VCR Modes

- **`VcrMode::Record`**: Always make real HTTP requests and record them. If an interaction already exists in the cassette, replay it instead.
- **`VcrMode::Replay`**: Only replay interactions from the cassette. Fail if no matching interaction is found.
- **`VcrMode::Once`**: Record interactions only if the cassette is empty, otherwise replay existing interactions.
- **`VcrMode::None`**: Pass through to the inner HTTP client without any recording or replaying.

## Request Matching

By default, requests are matched by HTTP method and URL. You can customize matching behavior:

```rust
use http_client_vcr::{DefaultMatcher, ExactMatcher};

// Custom matching (method, URL, specific headers)
let matcher = DefaultMatcher::new()
    .with_method(true)
    .with_url(true)
    .with_headers(vec!["Authorization".to_string(), "Content-Type".to_string()]);

let vcr_client = VcrClient::builder()
    .inner_client(inner_client)
    .cassette_path("fixtures/my_test.yaml")
    .matcher(Box::new(matcher))
    .build()
    .await?;
```

## Filtering Sensitive Data

VCR supports filtering sensitive data from requests and responses before they are stored in cassettes:

### Built-in Filters

```rust
use http_client_vcr::{HeaderFilter, BodyFilter, UrlFilter, FilterChain};

// Remove sensitive headers
let header_filter = HeaderFilter::new()
    .remove_auth_headers()  // Removes Authorization, Cookie, X-API-Key, etc.
    .remove_header("X-Custom-Secret")
    .replace_header("User-Id", "FILTERED");

// Filter JSON body content
let body_filter = BodyFilter::new()
    .remove_common_sensitive_keys()  // Removes password, token, api_key, etc.
    .remove_json_key("credit_card")
    .replace_regex(r"\d{4}-\d{4}-\d{4}-\d{4}", "XXXX-XXXX-XXXX-XXXX")
    .unwrap();

// Filter URL query parameters
let url_filter = UrlFilter::new()
    .remove_common_sensitive_params()  // Removes api_key, token, etc.
    .remove_query_param("secret")
    .replace_query_param("user_id", "FILTERED");

// Chain filters together
let filter_chain = FilterChain::new()
    .add_filter(Box::new(header_filter))
    .add_filter(Box::new(body_filter))
    .add_filter(Box::new(url_filter));

let vcr_client = VcrClient::builder()
    .inner_client(inner_client)
    .cassette_path("fixtures/filtered_test.yaml")
    .filter_chain(filter_chain)
    .build()
    .await?;
```

### Custom Filters

You can create custom filters for more complex scenarios:

```rust
use http_client_vcr::CustomFilter;

let custom_filter = CustomFilter::new(|req, resp| {
    // Remove any header containing "secret"
    req.headers.retain(|key, _| !key.to_lowercase().contains("secret"));
    
    // Replace response body if it contains errors
    if let Some(body) = &mut resp.body {
        if body.contains("error") {
            *body = r#"{"error": "FILTERED"}"#.to_string();
        }
    }
});

let vcr_client = VcrClient::builder()
    .inner_client(inner_client)
    .add_filter(Box::new(custom_filter))
    .build()
    .await?;
```

**Important: Filters are applied only to the data stored in cassette files, not to the actual HTTP interactions.** During recording:

1. **Real requests** are made with original sensitive data (so APIs work properly)
2. **Real responses** are returned to your application (unfiltered)  
3. **Filtered copies** are stored in the cassette (removing sensitive data)

This ensures your code gets the real data it needs while keeping cassettes safe for version control.

## NoOp Client for Testing

For ultimate safety during testing, VCR provides a `NoOpClient` that ensures no real HTTP requests are ever made:

```rust
use http_client_vcr::{VcrClient, VcrMode, NoOpClient};

// Guarantee no real HTTP requests can be made
let vcr_client = VcrClient::builder()
    .inner_client(Box::new(NoOpClient::new()))
    .cassette_path("tests/fixtures/api_test.yaml")
    .mode(VcrMode::Replay) // Only replay from cassette
    .build()
    .await?;

// This works if the request exists in the cassette
let response = vcr_client.send(request).await?;
// If not in cassette, you get a clear error (not a real HTTP request)
```

Two variants are available:

- **`NoOpClient::new()`** - Returns an error if a request is attempted
- **`NoOpClient::panicking()`** - Panics with a stack trace (useful for development)

This is particularly useful in CI/CD environments or when you want to be absolutely certain your tests are deterministic.

## Cassette Format

Cassettes are stored as YAML files with the following structure:

```yaml
interactions:
  - request:
      method: GET
      url: https://httpbin.org/get
      headers:
        User-Agent: ["http-client/1.0"]
      body: null
      version: Http1_1
    response:
      status: 200
      headers:
        Content-Type: ["application/json"]
      body: '{"origin": "127.0.0.1"}'
      version: Http1_1
```

## Testing with VCR

VCR is particularly useful for testing:

```rust
#[tokio::test]
async fn test_api_call() {
    let vcr_client = VcrClient::builder()
        .inner_client(Box::new(h1::H1Client::new()))
        .cassette_path("tests/fixtures/api_call.yaml")
        .mode(VcrMode::Once)
        .build()
        .await
        .unwrap();
    
    let request = Request::new(Method::Get, Url::parse("https://api.example.com/data").unwrap());
    let response = vcr_client.send(request).await.unwrap();
    
    assert_eq!(response.status(), 200);
    
    // First run records the interaction
    // Subsequent runs replay from cassette
}
```

## License

MIT