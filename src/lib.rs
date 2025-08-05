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

pub use cassette::{Cassette, CassetteFormat, Interaction};
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
    // Track which interactions have been used in replay mode (by index)
    used_interactions: Arc<Mutex<std::collections::HashSet<usize>>>,
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
            used_interactions: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// Synchronous version of directory save for use in Drop
    fn save_cassette_as_directory_sync(
        cassette: &Cassette,
        path: &PathBuf,
    ) -> Result<(), std::io::Error> {
        use serde::Serialize;

        // Create the cassette directory and bodies subdirectory
        std::fs::create_dir_all(path)?;
        let bodies_dir = path.join("bodies");
        std::fs::create_dir_all(&bodies_dir)?;

        // Create directory format structures for serialization
        #[derive(Serialize)]
        struct DirectoryInteraction {
            request: DirectorySerializableRequest,
            response: DirectorySerializableResponse,
        }

        #[derive(Serialize)]
        struct DirectorySerializableRequest {
            method: String,
            url: String,
            headers: std::collections::HashMap<String, Vec<String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            body_file: Option<String>,
            version: String,
        }

        #[derive(Serialize)]
        struct DirectorySerializableResponse {
            status: u16,
            headers: std::collections::HashMap<String, Vec<String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            body_file: Option<String>,
            version: String,
        }

        let mut dir_interactions = Vec::new();

        for (i, interaction) in cassette.interactions.iter().enumerate() {
            let interaction_num = format!("{:03}", i + 1);

            // Handle request body
            let request_body_file = if let Some(ref body) = interaction.request.body {
                if !body.is_empty() {
                    let filename = format!("req_{interaction_num}.txt");
                    let body_path = bodies_dir.join(&filename);
                    std::fs::write(&body_path, body)?;
                    Some(filename)
                } else {
                    None
                }
            } else if let Some(ref body_base64) = interaction.request.body_base64 {
                if !body_base64.is_empty() {
                    let filename = format!("req_{interaction_num}.b64");
                    let body_path = bodies_dir.join(&filename);
                    std::fs::write(&body_path, body_base64)?;
                    Some(filename)
                } else {
                    None
                }
            } else {
                None
            };

            // Handle response body
            let response_body_file = if let Some(ref body) = interaction.response.body {
                if !body.is_empty() {
                    let filename = format!("resp_{interaction_num}.txt");
                    let body_path = bodies_dir.join(&filename);
                    std::fs::write(&body_path, body)?;
                    Some(filename)
                } else {
                    None
                }
            } else if let Some(ref body_base64) = interaction.response.body_base64 {
                if !body_base64.is_empty() {
                    let filename = format!("resp_{interaction_num}.b64");
                    let body_path = bodies_dir.join(&filename);
                    std::fs::write(&body_path, body_base64)?;
                    Some(filename)
                } else {
                    None
                }
            } else {
                None
            };

            let dir_interaction = DirectoryInteraction {
                request: DirectorySerializableRequest {
                    method: interaction.request.method.clone(),
                    url: interaction.request.url.clone(),
                    headers: interaction.request.headers.clone(),
                    body_file: request_body_file,
                    version: interaction.request.version.clone(),
                },
                response: DirectorySerializableResponse {
                    status: interaction.response.status,
                    headers: interaction.response.headers.clone(),
                    body_file: response_body_file,
                    version: interaction.response.version.clone(),
                },
            };

            dir_interactions.push(dir_interaction);
        }

        // Write the interactions.yaml file
        let interactions_yaml = serde_yaml::to_string(&dir_interactions)
            .map_err(|e| std::io::Error::other(format!("Failed to serialize interactions: {e}")))?;

        let interactions_file = path.join("interactions.yaml");
        std::fs::write(&interactions_file, interactions_yaml)?;

        Ok(())
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
    ) -> Option<(usize, &'a Interaction)> {
        let used_interactions = self.used_interactions.lock().await;
        
        // Create a filtered copy of the request for matching against stored filtered interactions
        if let Ok(mut filtered_request) = SerializableRequest::from_request(request.clone()).await {
            self.filter_chain.filter_request(&mut filtered_request);

            cassette.interactions.iter().enumerate().find(|(index, interaction)| {
                !used_interactions.contains(index) && 
                self.matcher.matches_serializable(&filtered_request, &interaction.request)
            })
        } else {
            // Fallback to matching against stored interactions directly
            cassette
                .interactions
                .iter()
                .enumerate()
                .find(|(index, interaction)| {
                    !used_interactions.contains(index) && 
                    self.matcher.matches(request, &interaction.request)
                })
        }
    }

    /// Find similar URLs using Levenshtein distance when exact match fails
    async fn find_similar_urls(
        &self,
        request: &Request,
        cassette: &Cassette,
    ) -> Vec<(String, usize)> {
        let request_url = request.url().to_string();
        let mut similarities = Vec::new();

        for interaction in &cassette.interactions {
            let recorded_url = &interaction.request.url;
            let distance = levenshtein::levenshtein(&request_url, recorded_url);
            similarities.push((recorded_url.clone(), distance));
        }

        // Sort by distance (smaller distance = more similar)
        similarities.sort_by_key(|(_, distance)| *distance);

        // Return only the top 5 most similar URLs
        similarities.into_iter().take(5).collect()
    }

    /// Generate enhanced error message with URL similarity information
    async fn generate_no_match_error(&self, request: &Request, mode_description: &str) -> Error {
        let cassette = self.cassette.lock().await;
        let request_url = request.url().to_string();
        let request_method = request.method().to_string();

        let error_msg = {
            let mut msg = format!(
                "No matching interaction found in cassette ({mode_description})\n\nRequest details:\n  Method: {request_method}\n  URL: {request_url}"
            );

            if cassette.interactions.is_empty() {
                msg.push_str("\n\nCassette is empty - no recorded interactions available.");
            } else {
                msg.push_str(&format!(
                    "\n\nCassette contains {} recorded interactions.",
                    cassette.interactions.len()
                ));

                // Find similar URLs
                let similar_urls = self.find_similar_urls(request, &cassette).await;

                if !similar_urls.is_empty() {
                    msg.push_str("\n\nMost similar recorded URLs (by Levenshtein distance):");
                    for (i, (url, distance)) in similar_urls.iter().enumerate() {
                        msg.push_str(&format!("\n  {}. {} (distance: {})", i + 1, url, distance));
                    }
                }

                // Show unique methods in cassette
                let mut methods: Vec<String> = cassette
                    .interactions
                    .iter()
                    .map(|i| i.request.method.clone())
                    .collect();
                methods.sort();
                methods.dedup();

                msg.push_str(&format!("\n\nRecorded methods: {}", methods.join(", ")));
            }

            msg
        };

        // Convert to a static string by leaking memory (acceptable for error cases)
        Error::from_str(404, Box::leak(error_msg.into_boxed_str()))
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
        if let Some((index, _interaction)) = self.find_match(&req, &cassette).await {
            // Mark this interaction as used
            drop(cassette); // Release cassette lock before acquiring used_interactions lock
            let mut used_interactions = self.used_interactions.lock().await;
            used_interactions.insert(index);
            drop(used_interactions); // Release used_interactions lock
            
            // Re-acquire cassette lock to access the interaction
            let cassette = self.cassette.lock().await;
            let interaction = &cassette.interactions[index];
            Ok(interaction.response.to_response().await)
        } else {
            drop(cassette); // Release the lock before calling generate_no_match_error
            Err(self.generate_no_match_error(&req, "Replay mode").await)
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
        if let Some((index, _interaction)) = self.find_match(&req, &cassette).await {
            // Mark this interaction as used
            drop(cassette); // Release cassette lock before acquiring used_interactions lock
            let mut used_interactions = self.used_interactions.lock().await;
            used_interactions.insert(index);
            drop(used_interactions); // Release used_interactions lock
            
            // Re-acquire cassette lock to access the interaction
            let cassette = self.cassette.lock().await;
            let interaction = &cassette.interactions[index];
            return Ok(interaction.response.to_response().await);
        }

        if !cassette.is_empty() {
            drop(cassette); // Release the lock before calling generate_no_match_error
            return Err(self.generate_no_match_error(&req, "Once mode").await);
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
        if let Some((index, _interaction)) = self.find_match(&req, &cassette).await {
            // Mark this interaction as used
            drop(cassette); // Release cassette lock before acquiring used_interactions lock
            let mut used_interactions = self.used_interactions.lock().await;
            used_interactions.insert(index);
            drop(used_interactions); // Release used_interactions lock
            
            // Re-acquire cassette lock to access the interaction
            let cassette = self.cassette.lock().await;
            let interaction = &cassette.interactions[index];
            // Return the filtered response (filters are already applied when loading)
            Ok(interaction.response.to_response().await)
        } else {
            drop(cassette); // Release the lock before calling generate_no_match_error
            Err(self
                .generate_no_match_error(&req, "Filter mode - no new requests allowed")
                .await)
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
    format: Option<CassetteFormat>,
}

impl VcrClientBuilder {
    pub fn new<P: Into<PathBuf>>(cassette_path: P) -> Self {
        Self {
            inner: None,
            mode: VcrMode::Once,
            cassette_path: cassette_path.into(),
            matcher: None,
            filter_chain: FilterChain::new(),
            format: None,
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

    pub fn format(mut self, format: CassetteFormat) -> Self {
        self.format = Some(format);
        self
    }

    pub async fn build(self) -> Result<VcrClient, Error> {
        let inner = self
            .inner
            .ok_or_else(|| Error::from_str(400, "Inner HttpClient is required"))?;

        let cassette = if self.cassette_path.exists() {
            Cassette::load_from_file(self.cassette_path.clone()).await?
        } else {
            let mut cassette = Cassette::new().with_path(self.cassette_path);
            if let Some(format) = self.format {
                cassette = cassette.with_format(format);
            }
            cassette
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
                // Save respecting the format setting
                if let Some(path) = &cassette.path {
                    let result = match cassette.format {
                        CassetteFormat::File => {
                            // Save as single YAML file
                            if let Ok(yaml) = serde_yaml::to_string(&*cassette) {
                                std::fs::write(path, yaml)
                            } else {
                                Err(std::io::Error::other("Failed to serialize cassette"))
                            }
                        }
                        CassetteFormat::Directory => {
                            // Save as directory format (synchronous version)
                            Self::save_cassette_as_directory_sync(&cassette, path)
                        }
                    };

                    if let Err(e) = result {
                        eprintln!("Failed to save cassette on drop: {e}");
                    } else {
                        log::debug!("Successfully saved cassette to {path:?}");
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
