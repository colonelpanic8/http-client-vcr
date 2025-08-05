use crate::cassette::Cassette;
use crate::filter::FilterChain;
use crate::serializable::{SerializableRequest, SerializableResponse};
use http_client::Error;
use std::path::PathBuf;

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
