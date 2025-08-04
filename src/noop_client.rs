use async_trait::async_trait;
use http_client::{Config, Error, HttpClient, Request, Response};

/// A no-op HTTP client that always fails with an error.
///
/// This client is useful for testing VCR in replay mode to ensure
/// that no real HTTP requests are made. Any attempt to send a request
/// will result in an error.
///
/// ## Usage
///
/// ```rust,no_run
/// use http_client_vcr::{VcrClient, VcrMode, NoOpClient};
/// use http_client::HttpClient;
/// use http_types::{Request, Method, Url};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Ensure no real HTTP requests can be made
/// let vcr_client = VcrClient::builder("fixtures/test.yaml")
///     .inner_client(Box::new(NoOpClient::new()))
///     .mode(VcrMode::Replay) // Only replay from cassette
///     .build()
///     .await?;
///
/// // This will work if the request is in the cassette
/// let request = Request::new(Method::Get, Url::parse("https://example.com")?);
/// let response = vcr_client.send(request).await?;
///
/// // If not in cassette, VCR will return an error before reaching NoOpClient
/// // If somehow a request reaches NoOpClient, it will panic with a clear message
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct NoOpClient {
    error_message: String,
    config: Config,
}

impl NoOpClient {
    /// Create a new NoOpClient with a default error message.
    pub fn new() -> Self {
        Self {
            error_message: "NoOpClient: Real HTTP requests are not allowed. This indicates a VCR configuration issue - requests should be replayed from cassette.".to_string(),
            config: Config::new(),
        }
    }

    /// Create a NoOpClient with a custom error message.
    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            error_message: message.into(),
            config: Config::new(),
        }
    }

    /// Create a NoOpClient that panics instead of returning an error.
    /// This is useful for catching unexpected HTTP requests during development.
    pub fn panicking() -> PanickingNoOpClient {
        PanickingNoOpClient::new()
    }
}

impl Default for NoOpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for NoOpClient {
    async fn send(&self, req: Request) -> Result<Response, Error> {
        Err(Error::from_str(
            500,
            format!(
                "{} Attempted request: {} {}",
                self.error_message,
                req.method(),
                req.url()
            ),
        ))
    }

    fn set_config(&mut self, config: Config) -> Result<(), Error> {
        self.config = config;
        Ok(())
    }

    fn config(&self) -> &Config {
        &self.config
    }
}

/// A variant of NoOpClient that panics instead of returning an error.
///
/// This is useful during development to catch unexpected HTTP requests
/// with a clear stack trace showing exactly where the request originated.
#[derive(Debug, Clone)]
pub struct PanickingNoOpClient {
    panic_message: String,
    config: Config,
}

impl PanickingNoOpClient {
    pub fn new() -> Self {
        Self {
            panic_message: "PanickingNoOpClient: Unexpected HTTP request detected! This should not happen in VCR replay mode.".to_string(),
            config: Config::new(),
        }
    }

    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            panic_message: message.into(),
            config: Config::new(),
        }
    }
}

impl Default for PanickingNoOpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpClient for PanickingNoOpClient {
    async fn send(&self, req: Request) -> Result<Response, Error> {
        panic!(
            "{} Attempted request: {} {} - Check your VCR configuration and cassette contents.",
            self.panic_message,
            req.method(),
            req.url()
        );
    }

    fn set_config(&mut self, config: Config) -> Result<(), Error> {
        self.config = config;
        Ok(())
    }

    fn config(&self) -> &Config {
        &self.config
    }
}
