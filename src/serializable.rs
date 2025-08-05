use base64::{engine::general_purpose, Engine as _};
use http_client::{Error, Request, Response};
use http_types::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_base64: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableResponse {
    pub status: u16,
    pub headers: HashMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_base64: Option<String>,
    pub version: String,
}

impl SerializableRequest {
    pub async fn from_request(mut req: Request) -> Result<Self, Error> {
        let method = req.method().to_string();
        let url = req.url().to_string();
        let version = format!("{:?}", req.version());

        let mut headers = HashMap::new();
        for (name, values) in req.iter() {
            let header_values: Vec<String> =
                values.iter().map(|v| v.as_str().to_string()).collect();
            headers.insert(name.as_str().to_string(), header_values);
        }

        let (body, body_base64) = if req.len().is_some() {
            let body_string = req
                .body_string()
                .await
                .map_err(|e| Error::from_str(500, format!("Failed to read request body: {e}")))?;

            // Check if body contains binary/HTML content that should be base64 encoded
            if Self::should_base64_encode(&body_string) {
                (None, Some(general_purpose::STANDARD.encode(&body_string)))
            } else {
                (Some(body_string), None)
            }
        } else {
            (None, None)
        };

        Ok(Self {
            method,
            url,
            headers,
            body,
            body_base64,
            version,
        })
    }

    pub async fn to_request(&self) -> Result<Request, Error> {
        let method: Method = self
            .method
            .parse()
            .map_err(|e| Error::from_str(400, format!("Invalid method: {e}")))?;

        let url: Url = self
            .url
            .parse()
            .map_err(|e| Error::from_str(400, format!("Invalid URL: {e}")))?;

        let mut req = Request::new(method, url);

        for (name, values) in &self.headers {
            for value in values {
                let _ = req.append_header(name.as_str(), value.as_str());
            }
        }

        if let Some(body) = &self.body {
            req.set_body(body.clone());
        } else if let Some(body_base64) = &self.body_base64 {
            let decoded = general_purpose::STANDARD
                .decode(body_base64)
                .map_err(|e| Error::from_str(500, format!("Failed to decode base64 body: {e}")))?;
            let body_string = String::from_utf8(decoded).map_err(|e| {
                Error::from_str(
                    500,
                    format!("Failed to convert decoded body to string: {e}"),
                )
            })?;
            req.set_body(body_string);
        }

        Ok(req)
    }

    /// Determine if content should be base64 encoded to avoid YAML serialization issues
    fn should_base64_encode(content: &str) -> bool {
        // Base64 encode if content contains HTML tags, special YAML characters, or high ratio of non-ASCII
        content.contains('<') && content.contains('>') || // HTML content
        content.contains('%') && content.len() > 100 || // URL-encoded content
        content.chars().filter(|c| !c.is_ascii()).count() > content.len() / 10 // High non-ASCII ratio
    }
}

impl SerializableResponse {
    pub async fn from_response(mut res: Response) -> Result<Self, Error> {
        let status = res.status().into();
        let version = format!("{:?}", res.version());

        let mut headers = HashMap::new();
        for (name, values) in res.iter() {
            let header_values: Vec<String> =
                values.iter().map(|v| v.as_str().to_string()).collect();
            headers.insert(name.as_str().to_string(), header_values);
        }

        let (body, body_base64) = if res.len().is_some() {
            let body_string = res
                .body_string()
                .await
                .map_err(|e| Error::from_str(500, format!("Failed to read response body: {e}")))?;

            // Check if body contains binary/HTML content that should be base64 encoded
            if Self::should_base64_encode(&body_string) {
                (None, Some(general_purpose::STANDARD.encode(&body_string)))
            } else {
                (Some(body_string), None)
            }
        } else {
            (None, None)
        };

        Ok(Self {
            status,
            headers,
            body,
            body_base64,
            version,
        })
    }

    pub async fn to_response(&self) -> Response {
        let status = StatusCode::try_from(self.status).unwrap_or(StatusCode::InternalServerError);

        let mut res = Response::new(status);

        for (name, values) in &self.headers {
            for value in values {
                let _ = res.append_header(name.as_str(), value.as_str());
            }
        }

        if let Some(body) = &self.body {
            res.set_body(body.clone());
        } else if let Some(body_base64) = &self.body_base64 {
            if let Ok(decoded) = general_purpose::STANDARD.decode(body_base64) {
                if let Ok(body_string) = String::from_utf8(decoded) {
                    res.set_body(body_string);
                }
            }
        }

        res
    }

    /// Determine if content should be base64 encoded to avoid YAML serialization issues
    fn should_base64_encode(content: &str) -> bool {
        // Base64 encode if content contains HTML tags, special YAML characters, or high ratio of non-ASCII
        content.contains('<') && content.contains('>') || // HTML content
        content.contains('%') && content.len() > 100 || // URL-encoded content
        content.chars().filter(|c| !c.is_ascii()).count() > content.len() / 10 // High non-ASCII ratio
    }
}
