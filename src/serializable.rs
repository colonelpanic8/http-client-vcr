use http_client::{Error, Request, Response};
use http_types::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, Vec<String>>,
    pub body: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableResponse {
    pub status: u16,
    pub headers: HashMap<String, Vec<String>>,
    pub body: Option<String>,
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

        let body =
            if req.len().is_some() {
                Some(req.body_string().await.map_err(|e| {
                    Error::from_str(500, format!("Failed to read request body: {e}"))
                })?)
            } else {
                None
            };

        Ok(Self {
            method,
            url,
            headers,
            body,
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
        }

        Ok(req)
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

        let body =
            if res.len().is_some() {
                Some(res.body_string().await.map_err(|e| {
                    Error::from_str(500, format!("Failed to read response body: {e}"))
                })?)
            } else {
                None
            };

        Ok(Self {
            status,
            headers,
            body,
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
        }

        res
    }
}
