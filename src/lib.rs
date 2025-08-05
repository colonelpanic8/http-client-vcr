use async_trait::async_trait;
use http_client::{Error, HttpClient, Request, Response};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

mod cassette;
mod filter;
mod form_data;
mod matcher;
mod noop_client;
mod serializable;
mod utils;

pub use cassette::{Cassette, Interaction};
pub use filter::{
    BodyFilter, CustomFilter, Filter, FilterChain, HeaderFilter, SmartFormFilter, UrlFilter,
};
pub use form_data::{
    analyze_form_data, filter_form_data, find_credential_fields, parse_form_data, FormDataAnalysis,
};
pub use matcher::{DefaultMatcher, ExactMatcher, RequestMatcher};
pub use noop_client::{NoOpClient, PanickingNoOpClient};
pub use serializable::{SerializableRequest, SerializableResponse};
pub use utils::CassetteAnalysis;

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
    cassette: Arc<Mutex<Cassette>>,
    mode: VcrMode,
    matcher: Box<dyn RequestMatcher>,
    filter_chain: FilterChain,
    recording_started: Arc<Mutex<bool>>,
}

/// Duplicate a request while preserving the body.
///
/// Since Request::clone() sets the body to empty, this function properly
/// duplicates a request by reading the body into memory and restoring it
/// on both the original and cloned request.
///
/// Returns (request_for_sending, request_for_recording)
async fn duplicate_request_with_body(mut req: Request) -> Result<(Request, Request), Error> {
    // Read the body into bytes
    let body_bytes = req
        .take_body()
        .into_bytes()
        .await
        .map_err(|e| Error::from_str(500, format!("Failed to read request body: {e}")))?;

    // Clone the request (this gets everything except the body)
    let mut req_for_recording = req.clone();

    // Set the body on both requests (this creates two independent Body instances)
    req.set_body(body_bytes.clone());
    req_for_recording.set_body(body_bytes);

    Ok((req, req_for_recording))
}

impl VcrClient {
    pub fn new(inner: Box<dyn HttpClient>, mode: VcrMode, cassette: Cassette) -> Self {
        Self {
            inner,
            cassette: Arc::new(Mutex::new(cassette)),
            mode,
            matcher: Box::new(DefaultMatcher::new()),
            filter_chain: FilterChain::new(),
            recording_started: Arc::new(Mutex::new(false)),
        }
    }

    /// Create a pristine response from extracted data, completely independent of VCR processing
    fn create_pristine_response(
        status: http_types::StatusCode,
        headers: &std::collections::HashMap<String, Vec<String>>,
        body_content: Option<&str>,
    ) -> Response {
        let mut return_response = http_types::Response::new(status);

        // Copy all headers from the extracted header map
        for (name, values) in headers {
            for value in values {
                let _ = return_response.append_header(name.as_str(), value.as_str());
            }
        }

        // Set the body if we have content
        if let Some(body) = body_content {
            return_response.set_body(body);
        }

        return_response
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
        let cassette = self.cassette.lock().await;
        cassette.save_to_file().await
    }

    /// Apply filters to all interactions in the cassette
    /// This modifies the cassette in-place by applying the configured filter chain to all interactions
    pub async fn apply_filters_to_cassette(&self) -> Result<(), Error> {
        let mut cassette = self.cassette.lock().await;

        // Apply filters to each interaction
        for interaction in &mut cassette.interactions {
            self.filter_chain.filter_request(&mut interaction.request);
            self.filter_chain.filter_response(&mut interaction.response);
        }

        log::debug!(
            "Applied filters to {} interactions",
            cassette.interactions.len()
        );
        Ok(())
    }

    /// Apply filters to all interactions in the cassette and save the filtered version
    pub async fn filter_and_save_cassette(&self) -> Result<(), Error> {
        self.apply_filters_to_cassette().await?;
        self.save_cassette().await
    }

    pub fn builder<P: Into<PathBuf>>(cassette_path: P) -> VcrClientBuilder {
        VcrClientBuilder::new(cassette_path)
    }

    // Helper methods for each VCR mode

    /// Common logic for recording a request/response and returning the pristine response
    async fn record_and_return_response(
        &self,
        req_for_recording: Request,
        response: &mut Response,
    ) -> Result<Response, Error> {
        // IMMEDIATELY create a pristine copy for the caller before any VCR processing
        let status = response.status();
        let version = format!("{:?}", response.version());

        let mut headers = std::collections::HashMap::new();
        for (name, values) in response.iter() {
            let header_values: Vec<String> =
                values.iter().map(|v| v.as_str().to_string()).collect();
            headers.insert(name.as_str().to_string(), header_values);
        }

        // Read the body once - this consumes it from the original response
        let body_string = match response.body_string().await {
            Ok(body) if !body.is_empty() => Some(body),
            Ok(_) => None, // Empty body
            Err(e) => {
                // If we can't read the body, log it but don't fail the whole request
                eprintln!("Warning: Failed to read response body for VCR: {e}");
                None
            }
        };

        // Create the pristine return response immediately, before any VCR processing
        let return_response =
            Self::create_pristine_response(status, &headers, body_string.as_deref());

        // Now do VCR processing with the data we already extracted
        let mut serializable_request = SerializableRequest::from_request(req_for_recording).await?;
        let mut serializable_response = crate::SerializableResponse {
            status: status.into(),
            headers,
            body: body_string.clone(),
            body_base64: None,
            version,
        };

        // Apply filters ONLY to what gets stored
        self.filter_chain.filter_request(&mut serializable_request);
        self.filter_chain
            .filter_response(&mut serializable_response);

        let mut cassette = self.cassette.lock().await;

        // In Record mode, clear cassette on first interaction to fully replace
        if matches!(self.mode, VcrMode::Record) {
            let mut recording_started = self.recording_started.lock().await;
            if !*recording_started {
                cassette.clear();
                *recording_started = true;
            }
        }

        cassette
            .record_interaction(serializable_request, serializable_response)
            .await?;

        // Return the pristine response we created before any VCR processing
        Ok(return_response)
    }

    async fn handle_none_mode(&self, req: Request) -> Result<Response, Error> {
        self.inner.send(req).await
    }

    async fn handle_replay_mode(&self, req: Request) -> Result<Response, Error> {
        let cassette = self.cassette.lock().await;
        if let Some(interaction) = self.find_match(&req, &cassette).await {
            Ok(interaction.response.to_response().await)
        } else {
            Err(Error::from_str(
                404,
                "No matching interaction found in cassette",
            ))
        }
    }

    async fn handle_record_mode(&self, req: Request) -> Result<Response, Error> {
        // Duplicate the request to preserve the body for both sending and recording
        let (req_for_sending, req_for_recording) = duplicate_request_with_body(req).await?;

        // Make the real request with original sensitive data - never match existing interactions
        let mut response = self.inner.send(req_for_sending).await?;
        self.record_and_return_response(req_for_recording, &mut response)
            .await
    }

    async fn handle_once_mode(&self, req: Request) -> Result<Response, Error> {
        let cassette = self.cassette.lock().await;
        if let Some(interaction) = self.find_match(&req, &cassette).await {
            return Ok(interaction.response.to_response().await);
        }

        if !cassette.is_empty() {
            return Err(Error::from_str(
                404,
                "No matching interaction found in cassette (Once mode)",
            ));
        }
        drop(cassette); // Release the lock before making the request

        // Duplicate the request to preserve the body for both sending and recording
        let (req_for_sending, req_for_recording) = duplicate_request_with_body(req).await?;

        // Make the real request with original sensitive data
        let mut response = self.inner.send(req_for_sending).await?;
        self.record_and_return_response(req_for_recording, &mut response)
            .await
    }

    async fn handle_filter_mode(&self, req: Request) -> Result<Response, Error> {
        let cassette = self.cassette.lock().await;
        if let Some(interaction) = self.find_match(&req, &cassette).await {
            // Return the filtered response (filters are already applied when loading)
            Ok(interaction.response.to_response().await)
        } else {
            Err(Error::from_str(
                404,
                "No matching interaction found in cassette (Filter mode - no new requests allowed)",
            ))
        }
    }
}

// Re-export utility functions from the utils module
pub use utils::*;

#[derive(Debug)]
pub struct VcrClientBuilder {
    inner: Option<Box<dyn HttpClient>>,
    mode: VcrMode,
    cassette_path: PathBuf,
    matcher: Option<Box<dyn RequestMatcher>>,
    filter_chain: FilterChain,
}

impl VcrClientBuilder {
    pub fn new<P: Into<PathBuf>>(cassette_path: P) -> Self {
        Self {
            inner: None,
            mode: VcrMode::Once,
            cassette_path: cassette_path.into(),
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

        let cassette = if self.cassette_path.exists() {
            Cassette::load_from_file(self.cassette_path.clone()).await?
        } else {
            Cassette::new().with_path(self.cassette_path)
        };

        let mut vcr_client = VcrClient::new(inner, self.mode, cassette);

        if let Some(matcher) = self.matcher {
            vcr_client.set_matcher(matcher);
        }

        vcr_client.set_filter_chain(self.filter_chain);

        Ok(vcr_client)
    }
}

impl Drop for VcrClient {
    fn drop(&mut self) {
        if let Ok(cassette) = self.cassette.try_lock() {
            // Only save if:
            // 1. We're in a mode that should persist changes (Record or Once)
            // 2. The cassette was actually modified since loading
            let should_save = matches!(self.mode, VcrMode::Record | VcrMode::Once)
                && cassette.modified_since_load;

            if should_save {
                log::debug!(
                    "VcrClient dropped - saving modified cassette with {} interactions",
                    cassette.interactions.len()
                );
                // Try to save synchronously if possible
                if let Some(path) = &cassette.path {
                    if let Ok(yaml) = serde_yaml::to_string(&*cassette) {
                        if let Err(e) = std::fs::write(path, yaml) {
                            eprintln!("Failed to save cassette on drop: {e}");
                        } else {
                            log::debug!("Successfully saved cassette to {path:?}");
                        }
                    }
                }
            } else if cassette.modified_since_load {
                log::debug!(
                    "VcrClient dropped - not saving cassette (mode: {:?} doesn't persist changes)",
                    self.mode
                );
            }
        }
    }
}

#[async_trait]
impl HttpClient for VcrClient {
    async fn send(&self, req: Request) -> Result<Response, Error> {
        match &self.mode {
            VcrMode::None => self.handle_none_mode(req).await,
            VcrMode::Replay => self.handle_replay_mode(req).await,
            VcrMode::Record => self.handle_record_mode(req).await,
            VcrMode::Once => self.handle_once_mode(req).await,
            VcrMode::Filter => self.handle_filter_mode(req).await,
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
