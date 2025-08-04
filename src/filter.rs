use crate::serializable::{SerializableRequest, SerializableResponse};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt::Debug;

pub trait Filter: Debug + Send + Sync {
    fn filter_request(&self, request: &mut SerializableRequest);
    fn filter_response(&self, response: &mut SerializableResponse);
}

#[derive(Debug)]
pub struct FilterChain {
    filters: Vec<Box<dyn Filter>>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    pub fn add_filter(mut self, filter: Box<dyn Filter>) -> Self {
        self.filters.push(filter);
        self
    }

    pub fn filter_request(&self, request: &mut SerializableRequest) {
        for filter in &self.filters {
            filter.filter_request(request);
        }
    }

    pub fn filter_response(&self, response: &mut SerializableResponse) {
        for filter in &self.filters {
            filter.filter_response(response);
        }
    }
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct HeaderFilter {
    headers_to_remove: Vec<String>,
    headers_to_replace: HashMap<String, String>,
}

impl HeaderFilter {
    pub fn new() -> Self {
        Self {
            headers_to_remove: Vec::new(),
            headers_to_replace: HashMap::new(),
        }
    }

    pub fn remove_header(mut self, header: impl Into<String>) -> Self {
        self.headers_to_remove.push(header.into());
        self
    }

    pub fn replace_header(
        mut self,
        header: impl Into<String>,
        replacement: impl Into<String>,
    ) -> Self {
        self.headers_to_replace
            .insert(header.into(), replacement.into());
        self
    }

    pub fn remove_auth_headers(self) -> Self {
        self.remove_header("Authorization")
            .remove_header("Cookie")
            .remove_header("Set-Cookie")
            .remove_header("X-API-Key")
            .remove_header("X-Auth-Token")
    }

    fn filter_headers(&self, headers: &mut HashMap<String, Vec<String>>) {
        for header in &self.headers_to_remove {
            headers.remove(header);
        }

        for (header, replacement) in &self.headers_to_replace {
            if let Some(values) = headers.get_mut(header) {
                values.clear();
                values.push(replacement.clone());
            }
        }
    }
}

impl Filter for HeaderFilter {
    fn filter_request(&self, request: &mut SerializableRequest) {
        self.filter_headers(&mut request.headers);
    }

    fn filter_response(&self, response: &mut SerializableResponse) {
        self.filter_headers(&mut response.headers);
    }
}

impl Default for HeaderFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct BodyFilter {
    json_keys_to_remove: Vec<String>,
    json_keys_to_replace: HashMap<String, String>,
    regex_replacements: Vec<(Regex, String)>,
}

impl BodyFilter {
    pub fn new() -> Self {
        Self {
            json_keys_to_remove: Vec::new(),
            json_keys_to_replace: HashMap::new(),
            regex_replacements: Vec::new(),
        }
    }

    pub fn remove_json_key(mut self, key: impl Into<String>) -> Self {
        self.json_keys_to_remove.push(key.into());
        self
    }

    pub fn replace_json_key(
        mut self,
        key: impl Into<String>,
        replacement: impl Into<String>,
    ) -> Self {
        self.json_keys_to_replace
            .insert(key.into(), replacement.into());
        self
    }

    pub fn replace_regex(
        mut self,
        pattern: &str,
        replacement: impl Into<String>,
    ) -> Result<Self, regex::Error> {
        let regex = Regex::new(pattern)?;
        self.regex_replacements.push((regex, replacement.into()));
        Ok(self)
    }

    pub fn remove_common_sensitive_keys(self) -> Self {
        self.remove_json_key("password")
            .remove_json_key("token")
            .remove_json_key("api_key")
            .remove_json_key("secret")
            .remove_json_key("access_token")
            .remove_json_key("refresh_token")
            .remove_json_key("client_secret")
    }

    fn filter_json_object(&self, obj: &mut Map<String, Value>) {
        for key in &self.json_keys_to_remove {
            obj.remove(key);
        }

        for (key, replacement) in &self.json_keys_to_replace {
            if obj.contains_key(key) {
                obj.insert(key.clone(), Value::String(replacement.clone()));
            }
        }

        for (_, value) in obj.iter_mut() {
            self.filter_json_value(value);
        }
    }

    fn filter_json_value(&self, value: &mut Value) {
        match value {
            Value::Object(obj) => self.filter_json_object(obj),
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    self.filter_json_value(item);
                }
            }
            _ => {}
        }
    }

    fn filter_body(&self, body: &mut Option<String>) {
        if let Some(body_str) = body {
            if let Ok(mut json_value) = serde_json::from_str::<Value>(body_str) {
                self.filter_json_value(&mut json_value);
                if let Ok(filtered_json) = serde_json::to_string(&json_value) {
                    *body_str = filtered_json;
                }
            } else {
                for (regex, replacement) in &self.regex_replacements {
                    *body_str = regex.replace_all(body_str, replacement).to_string();
                }
            }
        }
    }
}

impl Filter for BodyFilter {
    fn filter_request(&self, request: &mut SerializableRequest) {
        self.filter_body(&mut request.body);
    }

    fn filter_response(&self, response: &mut SerializableResponse) {
        self.filter_body(&mut response.body);
    }
}

impl Default for BodyFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct UrlFilter {
    query_params_to_remove: Vec<String>,
    query_params_to_replace: HashMap<String, String>,
}

impl UrlFilter {
    pub fn new() -> Self {
        Self {
            query_params_to_remove: Vec::new(),
            query_params_to_replace: HashMap::new(),
        }
    }

    pub fn remove_query_param(mut self, param: impl Into<String>) -> Self {
        self.query_params_to_remove.push(param.into());
        self
    }

    pub fn replace_query_param(
        mut self,
        param: impl Into<String>,
        replacement: impl Into<String>,
    ) -> Self {
        self.query_params_to_replace
            .insert(param.into(), replacement.into());
        self
    }

    pub fn remove_common_sensitive_params(self) -> Self {
        self.remove_query_param("api_key")
            .remove_query_param("token")
            .remove_query_param("access_token")
            .remove_query_param("key")
    }
}

impl Filter for UrlFilter {
    fn filter_request(&self, request: &mut SerializableRequest) {
        if let Ok(mut url) = url::Url::parse(&request.url) {
            let mut query_pairs: Vec<(String, String)> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            query_pairs.retain(|(key, _)| !self.query_params_to_remove.contains(key));

            for (key, value) in &mut query_pairs {
                if let Some(replacement) = self.query_params_to_replace.get(key) {
                    *value = replacement.clone();
                }
            }

            url.query_pairs_mut().clear();
            for (key, value) in query_pairs {
                url.query_pairs_mut().append_pair(&key, &value);
            }

            request.url = url.to_string();
        }
    }

    fn filter_response(&self, _response: &mut SerializableResponse) {
        // URL filtering only applies to requests
    }
}

impl Default for UrlFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct CustomFilter<F>
where
    F: Fn(&mut SerializableRequest, &mut SerializableResponse) + Send + Sync + Debug,
{
    filter_fn: F,
}

impl<F> CustomFilter<F>
where
    F: Fn(&mut SerializableRequest, &mut SerializableResponse) + Send + Sync + Debug,
{
    pub fn new(filter_fn: F) -> Self {
        Self { filter_fn }
    }
}

impl<F> Filter for CustomFilter<F>
where
    F: Fn(&mut SerializableRequest, &mut SerializableResponse) + Send + Sync + Debug,
{
    fn filter_request(&self, request: &mut SerializableRequest) {
        let mut dummy_response = SerializableResponse {
            status: 200,
            headers: HashMap::new(),
            body: None,
            version: "Http1_1".to_string(),
        };
        (self.filter_fn)(request, &mut dummy_response);
    }

    fn filter_response(&self, response: &mut SerializableResponse) {
        let mut dummy_request = SerializableRequest {
            method: "GET".to_string(),
            url: "https://example.com".to_string(),
            headers: HashMap::new(),
            body: None,
            version: "Http1_1".to_string(),
        };
        (self.filter_fn)(&mut dummy_request, response);
    }
}
