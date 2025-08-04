use async_trait::async_trait;
use http_client::{Error, HttpClient, Request, Response};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

mod cassette;
mod filter;
mod lastfm_filters;
mod matcher;
mod noop_client;
mod serializable;

pub use cassette::{Cassette, Interaction};
pub use filter::{BodyFilter, CustomFilter, Filter, FilterChain, HeaderFilter, UrlFilter};
pub use lastfm_filters::{
    create_aggressive_lastfm_filter_chain, create_lastfm_filter_chain,
    create_minimal_lastfm_filter_chain,
};
pub use matcher::{DefaultMatcher, ExactMatcher, RequestMatcher};
pub use noop_client::{NoOpClient, PanickingNoOpClient};
pub use serializable::{SerializableRequest, SerializableResponse};

#[derive(Debug, Clone)]
pub enum VcrMode {
    Record,
    Replay,
    Once,
    None,
    Filter,
}

#[derive(Debug)]
pub struct VcrClient {
    inner: Box<dyn HttpClient>,
    cassette: Option<Arc<Mutex<Cassette>>>,
    mode: VcrMode,
    matcher: Box<dyn RequestMatcher>,
    filter_chain: FilterChain,
}

impl VcrClient {
    pub fn new(inner: Box<dyn HttpClient>, mode: VcrMode) -> Self {
        Self {
            inner,
            cassette: None,
            mode,
            matcher: Box::new(DefaultMatcher::new()),
            filter_chain: FilterChain::new(),
        }
    }

    pub fn with_cassette(mut self, cassette: Cassette) -> Self {
        self.cassette = Some(Arc::new(Mutex::new(cassette)));
        self
    }

    pub fn set_mode(&mut self, mode: VcrMode) {
        self.mode = mode;
    }

    pub fn set_matcher(&mut self, matcher: Box<dyn RequestMatcher>) {
        self.matcher = matcher;
    }

    pub fn set_filter_chain(&mut self, filter_chain: FilterChain) {
        self.filter_chain = filter_chain;
    }

    pub fn add_filter(&mut self, filter: Box<dyn Filter>) {
        self.filter_chain = std::mem::take(&mut self.filter_chain).add_filter(filter);
    }

    async fn find_match<'a>(
        &self,
        request: &Request,
        cassette: &'a Cassette,
    ) -> Option<&'a Interaction> {
        // Create a filtered copy of the request for matching against stored filtered interactions
        if let Ok(mut filtered_request) = SerializableRequest::from_request(request.clone()).await {
            self.filter_chain.filter_request(&mut filtered_request);

            cassette.interactions.iter().find(|interaction| {
                self.matcher
                    .matches_serializable(&filtered_request, &interaction.request)
            })
        } else {
            // Fallback to matching against stored interactions directly
            cassette
                .interactions
                .iter()
                .find(|interaction| self.matcher.matches(request, &interaction.request))
        }
    }

    pub async fn save_cassette(&self) -> Result<(), Error> {
        if let Some(cassette) = &self.cassette {
            let cassette = cassette.lock().await;
            cassette.save_to_file().await
        } else {
            Ok(())
        }
    }

    /// Apply filters to all interactions in the cassette
    /// This modifies the cassette in-place by applying the configured filter chain to all interactions
    pub async fn apply_filters_to_cassette(&self) -> Result<(), Error> {
        if let Some(cassette_arc) = &self.cassette {
            let mut cassette = cassette_arc.lock().await;

            // Apply filters to each interaction
            for interaction in &mut cassette.interactions {
                self.filter_chain.filter_request(&mut interaction.request);
                self.filter_chain.filter_response(&mut interaction.response);
            }

            println!(
                "Applied filters to {} interactions",
                cassette.interactions.len()
            );
            Ok(())
        } else {
            Err(Error::from_str(400, "No cassette loaded"))
        }
    }

    /// Apply filters to all interactions in the cassette and save the filtered version
    pub async fn filter_and_save_cassette(&self) -> Result<(), Error> {
        self.apply_filters_to_cassette().await?;
        self.save_cassette().await
    }

    pub fn builder() -> VcrClientBuilder {
        VcrClientBuilder::new()
    }
}

/// Utility function to apply filters to a cassette file and save the filtered version
/// This is useful for batch processing cassette files without creating a VcrClient
pub async fn filter_cassette_file<P: Into<PathBuf>>(
    cassette_path: P,
    filter_chain: FilterChain,
) -> Result<(), Error> {
    let path = cassette_path.into();

    // Load the cassette
    let mut cassette = Cassette::load_from_file(path.clone()).await?;

    // Apply filters to all interactions
    for interaction in &mut cassette.interactions {
        filter_chain.filter_request(&mut interaction.request);
        filter_chain.filter_response(&mut interaction.response);
    }

    // Save the filtered cassette
    cassette.save_to_file().await?;

    println!(
        "Applied filters to {} interactions in {path:?}",
        cassette.interactions.len()
    );
    Ok(())
}

#[derive(Debug)]
pub struct VcrClientBuilder {
    inner: Option<Box<dyn HttpClient>>,
    mode: VcrMode,
    cassette_path: Option<PathBuf>,
    matcher: Option<Box<dyn RequestMatcher>>,
    filter_chain: FilterChain,
}

impl VcrClientBuilder {
    pub fn new() -> Self {
        Self {
            inner: None,
            mode: VcrMode::Once,
            cassette_path: None,
            matcher: None,
            filter_chain: FilterChain::new(),
        }
    }

    pub fn inner_client(mut self, client: Box<dyn HttpClient>) -> Self {
        self.inner = Some(client);
        self
    }

    pub fn mode(mut self, mode: VcrMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn cassette_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.cassette_path = Some(path.into());
        self
    }

    pub fn matcher(mut self, matcher: Box<dyn RequestMatcher>) -> Self {
        self.matcher = Some(matcher);
        self
    }

    pub fn filter_chain(mut self, filter_chain: FilterChain) -> Self {
        self.filter_chain = filter_chain;
        self
    }

    pub fn add_filter(mut self, filter: Box<dyn Filter>) -> Self {
        self.filter_chain = self.filter_chain.add_filter(filter);
        self
    }

    pub async fn build(self) -> Result<VcrClient, Error> {
        let inner = self
            .inner
            .ok_or_else(|| Error::from_str(400, "Inner HttpClient is required"))?;

        let mut vcr_client = VcrClient::new(inner, self.mode);

        if let Some(matcher) = self.matcher {
            vcr_client.set_matcher(matcher);
        }

        vcr_client.set_filter_chain(self.filter_chain);

        if let Some(path) = self.cassette_path {
            let cassette = if path.exists() {
                Cassette::load_from_file(path.clone()).await?
            } else {
                Cassette::new().with_path(path)
            };

            vcr_client = vcr_client.with_cassette(cassette);
        }

        Ok(vcr_client)
    }
}

impl Default for VcrClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VcrClient {
    fn drop(&mut self) {
        if let Some(cassette_arc) = &self.cassette {
            if let Ok(cassette) = cassette_arc.try_lock() {
                println!(
                    "VcrClient dropped - trying to save cassette with {} interactions",
                    cassette.interactions.len()
                );
                // Try to save synchronously if possible
                if let Some(path) = &cassette.path {
                    if let Ok(yaml) = serde_yaml::to_string(&*cassette) {
                        if let Err(e) = std::fs::write(path, yaml) {
                            eprintln!("Failed to save cassette on drop: {e}");
                        } else {
                            println!("Successfully saved cassette to {path:?}");
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl HttpClient for VcrClient {
    async fn send(&self, req: Request) -> Result<Response, Error> {
        match &self.mode {
            VcrMode::None => self.inner.send(req).await,
            VcrMode::Replay => {
                if let Some(cassette_arc) = &self.cassette {
                    let cassette = cassette_arc.lock().await;
                    if let Some(interaction) = self.find_match(&req, &cassette).await {
                        return Ok(interaction.response.to_response().await);
                    }
                }
                Err(Error::from_str(
                    404,
                    "No matching interaction found in cassette",
                ))
            }
            VcrMode::Record => {
                if let Some(cassette_arc) = &self.cassette {
                    let cassette = cassette_arc.lock().await;
                    if let Some(interaction) = self.find_match(&req, &cassette).await {
                        return Ok(interaction.response.to_response().await);
                    }
                }

                // Make the real request with original sensitive data
                let mut response = self.inner.send(req.clone()).await?;

                // Store filtered copies in cassette
                if let Some(cassette_arc) = &self.cassette {
                    let mut serializable_request = SerializableRequest::from_request(req).await?;

                    // Read the response body once and share it
                    let status = response.status();
                    let version = format!("{:?}", response.version());

                    let mut headers = std::collections::HashMap::new();
                    for (name, values) in response.iter() {
                        let header_values: Vec<String> =
                            values.iter().map(|v| v.as_str().to_string()).collect();
                        headers.insert(name.as_str().to_string(), header_values);
                    }

                    // Always try to read the body - response.len() may be None for gzipped content
                    let body_string = match response.body_string().await {
                        Ok(body) if !body.is_empty() => Some(body),
                        Ok(_) => None, // Empty body
                        Err(e) => {
                            // If we can't read the body, log it but don't fail the whole request
                            eprintln!("Warning: Failed to read response body for VCR: {e}");
                            None
                        }
                    };

                    // Create serializable response with the body we just read
                    let mut serializable_response = crate::SerializableResponse {
                        status: status.into(),
                        headers,
                        body: body_string.clone(),
                        version,
                    };

                    // Apply filters ONLY to what gets stored
                    self.filter_chain.filter_request(&mut serializable_request);
                    self.filter_chain
                        .filter_response(&mut serializable_response);

                    let mut cassette = cassette_arc.lock().await;
                    cassette
                        .record_interaction(serializable_request, serializable_response)
                        .await?;

                    // Create a new response with the body we read to return to the caller
                    let mut return_response = http_types::Response::new(status);
                    for (name, values) in response.iter() {
                        for value in values {
                            let _ = return_response.append_header(name.as_str(), value.as_str());
                        }
                    }
                    if let Some(body) = body_string {
                        return_response.set_body(body);
                    }

                    return Ok(return_response);
                }

                // Return the original response if no cassette
                Ok(response)
            }
            VcrMode::Once => {
                if let Some(cassette_arc) = &self.cassette {
                    let cassette = cassette_arc.lock().await;
                    if let Some(interaction) = self.find_match(&req, &cassette).await {
                        return Ok(interaction.response.to_response().await);
                    }

                    if !cassette.is_empty() {
                        return Err(Error::from_str(
                            404,
                            "No matching interaction found in cassette (Once mode)",
                        ));
                    }
                }

                // Make the real request with original sensitive data
                let mut response = self.inner.send(req.clone()).await?;

                // Store filtered copies in cassette
                if let Some(cassette_arc) = &self.cassette {
                    let mut serializable_request = SerializableRequest::from_request(req).await?;

                    // Read the response body once and share it
                    let status = response.status();
                    let version = format!("{:?}", response.version());

                    let mut headers = std::collections::HashMap::new();
                    for (name, values) in response.iter() {
                        let header_values: Vec<String> =
                            values.iter().map(|v| v.as_str().to_string()).collect();
                        headers.insert(name.as_str().to_string(), header_values);
                    }

                    // Always try to read the body - response.len() may be None for gzipped content
                    let body_string = match response.body_string().await {
                        Ok(body) if !body.is_empty() => Some(body),
                        Ok(_) => None, // Empty body
                        Err(e) => {
                            // If we can't read the body, log it but don't fail the whole request
                            eprintln!("Warning: Failed to read response body for VCR: {e}");
                            None
                        }
                    };

                    // Create serializable response with the body we just read
                    let mut serializable_response = crate::SerializableResponse {
                        status: status.into(),
                        headers,
                        body: body_string.clone(),
                        version,
                    };

                    // Apply filters ONLY to what gets stored
                    self.filter_chain.filter_request(&mut serializable_request);
                    self.filter_chain
                        .filter_response(&mut serializable_response);

                    let mut cassette = cassette_arc.lock().await;
                    cassette
                        .record_interaction(serializable_request, serializable_response)
                        .await?;

                    // Create a new response with the body we read to return to the caller
                    let mut return_response = http_types::Response::new(status);
                    for (name, values) in response.iter() {
                        for value in values {
                            let _ = return_response.append_header(name.as_str(), value.as_str());
                        }
                    }
                    if let Some(body) = body_string {
                        return_response.set_body(body);
                    }

                    return Ok(return_response);
                }

                // Return the original response if no cassette
                Ok(response)
            }
            VcrMode::Filter => {
                if let Some(cassette_arc) = &self.cassette {
                    let cassette = cassette_arc.lock().await;
                    if let Some(interaction) = self.find_match(&req, &cassette).await {
                        // Return the filtered response (filters are already applied when loading)
                        return Ok(interaction.response.to_response().await);
                    }
                }
                Err(Error::from_str(
                    404,
                    "No matching interaction found in cassette (Filter mode - no new requests allowed)",
                ))
            }
        }
    }

    fn set_config(&mut self, config: http_client::Config) -> Result<(), Error> {
        self.inner
            .set_config(config)
            .map_err(|e| Error::from_str(500, format!("Config error: {e}")))
    }

    fn config(&self) -> &http_client::Config {
        self.inner.config()
    }
}
