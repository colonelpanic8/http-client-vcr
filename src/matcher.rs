use crate::serializable::SerializableRequest;
use http_client::Request;
use std::fmt::Debug;

pub trait RequestMatcher: Debug + Send + Sync {
    fn matches(&self, request: &Request, recorded_request: &SerializableRequest) -> bool;

    fn matches_serializable(
        &self,
        request: &SerializableRequest,
        recorded_request: &SerializableRequest,
    ) -> bool {
        // Default implementation compares serialized forms
        request.method == recorded_request.method && request.url == recorded_request.url
    }
}

#[derive(Debug)]
pub struct DefaultMatcher {
    match_method: bool,
    match_url: bool,
    match_headers: Vec<String>,
    match_body: bool,
}

impl DefaultMatcher {
    pub fn new() -> Self {
        Self {
            match_method: true,
            match_url: true,
            // By default, match common headers including cookies - this is the correct behavior
            match_headers: vec![
                "authorization".to_string(),
                "cookie".to_string(),
                "content-type".to_string(),
                "user-agent".to_string(),
            ],
            match_body: false,
        }
    }

    /// Create a matcher that ignores cookies - useful for tests where cookies change
    pub fn without_cookies() -> Self {
        Self {
            match_method: true,
            match_url: true,
            match_headers: vec![
                "authorization".to_string(),
                "content-type".to_string(),
                "user-agent".to_string(),
            ],
            match_body: false,
        }
    }

    pub fn with_method(mut self, match_method: bool) -> Self {
        self.match_method = match_method;
        self
    }

    pub fn with_url(mut self, match_url: bool) -> Self {
        self.match_url = match_url;
        self
    }

    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.match_headers = headers;
        self
    }

    pub fn with_body(mut self, match_body: bool) -> Self {
        self.match_body = match_body;
        self
    }
}

impl RequestMatcher for DefaultMatcher {
    fn matches(&self, request: &Request, recorded_request: &SerializableRequest) -> bool {
        log::debug!(
            "Matching request: {} {} against recorded: {} {}",
            request.method(),
            request.url(),
            recorded_request.method,
            recorded_request.url
        );

        if self.match_method && request.method().to_string() != recorded_request.method {
            log::debug!(
                "Method mismatch: {} != {}",
                request.method(),
                recorded_request.method
            );
            return false;
        }

        if self.match_url && request.url().to_string() != recorded_request.url {
            log::debug!(
                "URL mismatch: {} != {}",
                request.url(),
                recorded_request.url
            );
            return false;
        }

        if !self.match_headers.is_empty() {
            log::debug!("Checking {} headers for matching", self.match_headers.len());
            for header_name in &self.match_headers {
                let request_header = request.header(header_name.as_str());
                let recorded_header = recorded_request.headers.get(header_name);

                log::debug!(
                    "Comparing header '{}': request={:?}, recorded={:?}",
                    header_name,
                    request_header.map(|v| v.iter().map(|h| h.as_str()).collect::<Vec<_>>()),
                    recorded_header
                );

                match (request_header, recorded_header) {
                    (Some(req_val), Some(rec_val)) => {
                        let req_values: Vec<String> =
                            req_val.iter().map(|v| v.as_str().to_string()).collect();
                        if &req_values != rec_val {
                            log::debug!(
                                "Header '{}' values mismatch: request={:?} != recorded={:?}",
                                header_name,
                                req_values,
                                rec_val
                            );
                            return false;
                        } else {
                            log::debug!("Header '{}' matched: {:?}", header_name, req_values);
                        }
                    }
                    (None, None) => {
                        log::debug!("Header '{}' both absent (matched)", header_name);
                    }
                    _ => {
                        log::debug!("Header '{}' presence mismatch: request present={}, recorded present={}", 
                                   header_name, request_header.is_some(), recorded_header.is_some());
                        return false;
                    }
                }
            }
        }

        log::debug!("Request matched successfully");
        true
    }

    fn matches_serializable(
        &self,
        request: &SerializableRequest,
        recorded_request: &SerializableRequest,
    ) -> bool {
        if self.match_method && request.method != recorded_request.method {
            return false;
        }

        if self.match_url && request.url != recorded_request.url {
            return false;
        }

        if !self.match_headers.is_empty() {
            for header_name in &self.match_headers {
                let request_header = request.headers.get(header_name);
                let recorded_header = recorded_request.headers.get(header_name);

                match (request_header, recorded_header) {
                    (Some(req_val), Some(rec_val)) => {
                        if req_val != rec_val {
                            return false;
                        }
                    }
                    (None, None) => {}
                    _ => return false,
                }
            }
        }

        true
    }
}

impl Default for DefaultMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct ExactMatcher;

impl RequestMatcher for ExactMatcher {
    fn matches(&self, request: &Request, recorded_request: &SerializableRequest) -> bool {
        if request.method().to_string() != recorded_request.method {
            return false;
        }

        if request.url().to_string() != recorded_request.url {
            return false;
        }

        let mut request_headers = std::collections::HashMap::new();
        for (name, values) in request.iter() {
            let header_values: Vec<String> =
                values.iter().map(|v| v.as_str().to_string()).collect();
            request_headers.insert(name.as_str().to_string(), header_values);
        }

        if request_headers != recorded_request.headers {
            return false;
        }

        true
    }

    fn matches_serializable(
        &self,
        request: &SerializableRequest,
        recorded_request: &SerializableRequest,
    ) -> bool {
        request.method == recorded_request.method
            && request.url == recorded_request.url
            && request.headers == recorded_request.headers
    }
}
