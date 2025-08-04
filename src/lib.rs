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

// Note: All the helper functions for mutating cassettes are defined below and automatically public

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

    log::debug!(
        "Applied filters to {} interactions in {path:?}",
        cassette.interactions.len()
    );
    Ok(())
}

/// Apply a filter function to all requests in a cassette file
/// This allows for custom mutation logic beyond the standard filter chains
pub async fn mutate_all_requests<P, F>(cassette_path: P, mut mutator: F) -> Result<(), Error>
where
    P: Into<PathBuf>,
    F: FnMut(&mut SerializableRequest),
{
    let path = cassette_path.into();
    let mut cassette = Cassette::load_from_file(path.clone()).await?;

    for interaction in &mut cassette.interactions {
        mutator(&mut interaction.request);
    }

    cassette.save_to_file().await?;
    log::debug!(
        "Applied custom mutations to {} requests in {path:?}",
        cassette.interactions.len()
    );
    Ok(())
}

/// Apply a filter function to all responses in a cassette file
pub async fn mutate_all_responses<P, F>(cassette_path: P, mut mutator: F) -> Result<(), Error>
where
    P: Into<PathBuf>,
    F: FnMut(&mut SerializableResponse),
{
    let path = cassette_path.into();
    let mut cassette = Cassette::load_from_file(path.clone()).await?;

    for interaction in &mut cassette.interactions {
        mutator(&mut interaction.response);
    }

    cassette.save_to_file().await?;
    log::debug!(
        "Applied custom mutations to {} responses in {path:?}",
        cassette.interactions.len()
    );
    Ok(())
}

/// Apply mutation functions to both requests and responses in a cassette file
pub async fn mutate_all_interactions<P, RF, ResF>(
    cassette_path: P,
    mut request_mutator: RF,
    mut response_mutator: ResF,
) -> Result<(), Error>
where
    P: Into<PathBuf>,
    RF: FnMut(&mut SerializableRequest),
    ResF: FnMut(&mut SerializableResponse),
{
    let path = cassette_path.into();
    let mut cassette = Cassette::load_from_file(path.clone()).await?;

    for interaction in &mut cassette.interactions {
        request_mutator(&mut interaction.request);
        response_mutator(&mut interaction.response);
    }

    cassette.save_to_file().await?;
    log::debug!(
        "Applied custom mutations to {} interactions in {path:?}",
        cassette.interactions.len()
    );
    Ok(())
}

/// Helper to remove all sensitive form data from requests using smart detection
pub async fn strip_all_credentials_from_requests<P: Into<PathBuf>>(
    cassette_path: P,
) -> Result<(), Error> {
    mutate_all_requests(cassette_path, |request| {
        if let Some(body) = &mut request.body {
            // Check if this looks like form data
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let filtered = crate::form_data::filter_form_data(body, "[REMOVED]");
                *body = filtered;
            }
        }
    })
    .await
}

/// Helper to remove all cookie headers from requests and set-cookie from responses
pub async fn strip_all_cookies<P: Into<PathBuf>>(cassette_path: P) -> Result<(), Error> {
    mutate_all_interactions(
        cassette_path,
        |request| {
            request.headers.remove("cookie");
            request.headers.remove("Cookie");
        },
        |response| {
            response.headers.remove("set-cookie");
            response.headers.remove("Set-Cookie");
        },
    )
    .await
}

/// Replace specific field values in all form data requests
pub async fn replace_form_field_in_all_requests<P: Into<PathBuf>>(
    cassette_path: P,
    field_name: &str,
    replacement_value: &str,
) -> Result<(), Error> {
    let field = field_name.to_string();
    let replacement = replacement_value.to_string();

    mutate_all_requests(cassette_path, move |request| {
        if let Some(body) = &mut request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let mut params = crate::form_data::parse_form_data(body);
                if params.contains_key(&field) {
                    params.insert(field.clone(), replacement.clone());
                    *body = crate::form_data::encode_form_data(&params);
                }
            }
        }
    })
    .await
}

/// Remove specific header from all requests
pub async fn remove_header_from_all_requests<P: Into<PathBuf>>(
    cassette_path: P,
    header_name: &str,
) -> Result<(), Error> {
    let header = header_name.to_string();

    mutate_all_requests(cassette_path, move |request| {
        request.headers.remove(&header);
        // Also try lowercase version
        request.headers.remove(&header.to_lowercase());
    })
    .await
}

/// Replace specific header value in all requests
pub async fn replace_header_in_all_requests<P: Into<PathBuf>>(
    cassette_path: P,
    header_name: &str,
    replacement_value: &str,
) -> Result<(), Error> {
    let header = header_name.to_string();
    let replacement = replacement_value.to_string();

    mutate_all_requests(cassette_path, move |request| {
        if request.headers.contains_key(&header) {
            request
                .headers
                .insert(header.clone(), vec![replacement.clone()]);
        }
        // Also check lowercase version
        let header_lower = header.to_lowercase();
        if request.headers.contains_key(&header_lower) {
            request
                .headers
                .insert(header_lower, vec![replacement.clone()]);
        }
    })
    .await
}

/// Scrub URLs by removing or replacing query parameters
pub async fn scrub_urls_in_all_requests<P: Into<PathBuf>, F>(
    cassette_path: P,
    mut url_mutator: F,
) -> Result<(), Error>
where
    F: FnMut(&str) -> String,
{
    mutate_all_requests(cassette_path, move |request| {
        request.url = url_mutator(&request.url);
    })
    .await
}

/// Helper to replace all instances of a specific username across all requests
pub async fn replace_username_in_all_requests<P: Into<PathBuf>>(
    cassette_path: P,
    new_username: &str,
) -> Result<(), Error> {
    let replacement = new_username.to_string();

    mutate_all_requests(cassette_path, move |request| {
        // Handle form data
        if let Some(body) = &mut request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let mut params = crate::form_data::parse_form_data(body);

                // Look for common username fields
                let username_fields = ["username", "user", "username_or_email", "email", "login"];
                for field in &username_fields {
                    if params.contains_key(*field) {
                        params.insert(field.to_string(), replacement.clone());
                    }
                }

                *body = crate::form_data::encode_form_data(&params);
            }
        }

        // Handle basic auth in headers
        if let Some(auth_headers) = request.headers.get_mut("authorization") {
            for auth_header in auth_headers.iter_mut() {
                if auth_header.starts_with("Basic ") {
                    // For basic auth, we'd need to decode, replace username, re-encode
                    // For now, just replace the whole thing
                    *auth_header = "[FILTERED_BASIC_AUTH]".to_string();
                }
            }
        }
    })
    .await
}

/// One-stop function to sanitize an entire cassette for sharing/testing
pub async fn sanitize_cassette_for_sharing<P: Into<PathBuf>>(
    cassette_path: P,
) -> Result<(), Error> {
    let path = cassette_path.into();

    log::debug!("🧹 Sanitizing cassette for sharing: {path:?}");

    // First analyze what we're dealing with
    let analysis = analyze_cassette_file(&path).await?;
    analysis.print_report();

    log::debug!("\n🔧 Applying sanitization...");

    // Apply comprehensive cleaning
    mutate_all_interactions(
        &path,
        |request| {
            // Clean headers
            request.headers.remove("authorization");
            request.headers.remove("Authorization");

            // Clean form data
            if let Some(body) = &mut request.body {
                if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                    *body = crate::form_data::filter_form_data(body, "[SANITIZED]");
                }
            }

            // Clean URLs of sensitive query params
            if let Ok(mut url) = url::Url::parse(&request.url) {
                let sensitive_params = ["api_key", "access_token", "key"];
                let query_pairs: Vec<(String, String)> = url
                    .query_pairs()
                    .filter(|(key, _)| !sensitive_params.contains(&key.as_ref()))
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();

                url.query_pairs_mut().clear();
                for (key, value) in query_pairs {
                    url.query_pairs_mut().append_pair(&key, &value);
                }

                request.url = url.to_string();
            }
        },
        |response| {
            // Clean response headers

            // Clean sensitive data from response bodies
            if let Some(body) = &mut response.body {
                // Simple replacements for common sensitive patterns
                *body = body.replace(r#""sessionid":"[^"]*""#, r#""sessionid":"[SANITIZED]""#);
            }
        },
    )
    .await?;

    log::debug!("✅ Cassette sanitized successfully!");
    log::debug!("🔒 All credentials, session data, and sensitive headers have been removed");

    Ok(())
}

/// Analyze a cassette file for sensitive data without modifying it
/// This helps identify what needs to be filtered
pub async fn analyze_cassette_file<P: Into<PathBuf>>(
    cassette_path: P,
) -> Result<CassetteAnalysis, Error> {
    let path = cassette_path.into();
    let cassette = Cassette::load_from_file(path.clone()).await?;

    let mut analysis = CassetteAnalysis {
        file_path: path,
        total_interactions: cassette.interactions.len(),
        requests_with_form_data: Vec::new(),
        requests_with_credentials: Vec::new(),
        sensitive_headers: Vec::new(),
    };

    for (i, interaction) in cassette.interactions.iter().enumerate() {
        // Analyze request body for form data
        if let Some(body) = &interaction.request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let form_analysis = crate::form_data::analyze_form_data(body);
                if !form_analysis.credential_fields.is_empty() {
                    analysis.requests_with_form_data.push(i);
                    analysis
                        .requests_with_credentials
                        .push((i, form_analysis.credential_fields));
                }
            }
        }

        // Analyze headers for sensitive data
        for (header_name, header_values) in &interaction.request.headers {
            let header_lower = header_name.to_lowercase();
            if header_lower.contains("cookie")
                || header_lower.contains("authorization")
                || header_lower.contains("token")
            {
                analysis
                    .sensitive_headers
                    .push((i, header_name.clone(), header_values.clone()));
            }
        }

        // Also check response headers
        for (header_name, header_values) in &interaction.response.headers {
            let header_lower = header_name.to_lowercase();
            if header_lower.contains("set-cookie")
                || header_lower.contains("authorization")
                || header_lower.contains("token")
            {
                analysis.sensitive_headers.push((
                    i,
                    format!("response-{header_name}"),
                    header_values.clone(),
                ));
            }
        }
    }

    Ok(analysis)
}

/// Replace the password in all requests with a test password
/// This is useful when you want to use a known test password for replay
pub async fn set_test_password_in_cassette<P: Into<PathBuf>>(
    cassette_path: P,
    test_password: &str,
) -> Result<(), Error> {
    let path = cassette_path.into();
    let password = test_password.to_string();

    log::debug!("🔑 Setting test password in cassette: {path:?}");

    mutate_all_requests(&path, move |request| {
        if let Some(body) = &mut request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let mut params = crate::form_data::parse_form_data(body);

                if params.contains_key("password") {
                    params.insert("password".to_string(), password.clone());
                    *body = crate::form_data::encode_form_data(&params);
                }
            }
        }
    })
    .await?;

    log::debug!("✅ Test password set in cassette");
    Ok(())
}

/// Get the username from a cassette (useful for test setup)
/// Returns the first username found in form data
pub async fn extract_username_from_cassette<P: Into<PathBuf>>(
    cassette_path: P,
) -> Result<Option<String>, Error> {
    let path = cassette_path.into();
    let cassette = Cassette::load_from_file(path).await?;

    for interaction in &cassette.interactions {
        if let Some(body) = &interaction.request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let params = crate::form_data::parse_form_data(body);

                // Look for common username fields
                let username_fields = ["username", "username_or_email", "user", "email"];
                for field in &username_fields {
                    if let Some(username) = params.get(*field) {
                        // Skip filtered values
                        if !username.starts_with("[FILTERED") && !username.starts_with("[SANITIZED")
                        {
                            return Ok(Some(username.clone()));
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

#[derive(Debug)]
pub struct CassetteAnalysis {
    pub file_path: PathBuf,
    pub total_interactions: usize,
    pub requests_with_form_data: Vec<usize>,
    pub requests_with_credentials: Vec<(usize, Vec<(String, String)>)>,
    pub sensitive_headers: Vec<(usize, String, Vec<String>)>,
}

impl CassetteAnalysis {
    /// Print a detailed analysis report
    pub fn print_report(&self) {
        log::debug!("📊 Cassette Analysis Report");
        log::debug!("=====================================");
        log::debug!("File: {:?}", self.file_path);
        log::debug!("Total interactions: {}", self.total_interactions);
        log::debug!("");

        if !self.requests_with_form_data.is_empty() {
            log::debug!(
                "🔍 Interactions with form data: {}",
                self.requests_with_form_data.len()
            );
            for idx in &self.requests_with_form_data {
                log::debug!("  - Interaction #{idx}");
            }
            log::debug!("");
        }

        if !self.requests_with_credentials.is_empty() {
            log::debug!(
                "🔐 Interactions containing credentials: {}",
                self.requests_with_credentials.len()
            );
            for (idx, credentials) in &self.requests_with_credentials {
                log::debug!(
                    "  - Interaction #{}: {} credential fields",
                    idx,
                    credentials.len()
                );
                for (key, value) in credentials {
                    let preview = if value.len() > 20 {
                        format!("{}...", &value[..20])
                    } else {
                        value.clone()
                    };
                    log::debug!("    * {key}: {preview}");
                }
            }
            log::debug!("");
        }

        if !self.sensitive_headers.is_empty() {
            log::debug!(
                "🏷️  Interactions with sensitive headers: {}",
                self.sensitive_headers.len()
            );
            for (idx, header_name, header_values) in &self.sensitive_headers {
                log::debug!("  - Interaction #{idx}: {header_name} header");
                for value in header_values {
                    let preview = if value.len() > 50 {
                        format!("{}...", &value[..50])
                    } else {
                        value.clone()
                    };
                    log::debug!("    * {preview}");
                }
            }
            log::debug!("");
        }

        log::debug!("💡 Recommendations:");
        if !self.requests_with_credentials.is_empty() {
            log::debug!(
                "  - Use SmartFormFilter to automatically detect and filter form credentials"
            );
        }
        if !self.sensitive_headers.is_empty() {
            log::debug!("  - Use HeaderFilter to filter sensitive headers like cookies and tokens");
        }
        if self.requests_with_form_data.is_empty() && self.sensitive_headers.is_empty() {
            log::debug!("  - No obvious sensitive data detected, but consider reviewing manually");
        }
    }
}

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
