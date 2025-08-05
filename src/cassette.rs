use crate::serializable::{SerializableRequest, SerializableResponse};
use http_client::Error;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    pub request: SerializableRequest,
    pub response: SerializableResponse,
}

#[derive(Debug, Clone, Default)]
pub enum CassetteFormat {
    /// Traditional single YAML file format
    #[default]
    File,
    /// Directory format with separate body files
    Directory,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Cassette {
    pub interactions: Vec<Interaction>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
    #[serde(skip)]
    pub modified_since_load: bool,
    #[serde(skip)]
    pub format: CassetteFormat,
}

impl Cassette {
    pub fn new() -> Self {
        Self {
            interactions: Vec::new(),
            path: None,
            modified_since_load: false,
            format: CassetteFormat::File, // Default to file format
        }
    }

    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    /// Explicitly set the cassette format (useful when creating new cassettes)
    pub fn with_format(mut self, format: CassetteFormat) -> Self {
        self.format = format;
        self
    }

    pub async fn load_from_file(path: PathBuf) -> Result<Self, Error> {
        // Simple detection: if it's a directory, load as directory format, otherwise as file
        if path.is_dir() {
            Self::load_from_directory(path).await
        } else {
            Self::load_from_single_file(path).await
        }
    }

    async fn load_from_single_file(path: PathBuf) -> Result<Self, Error> {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| Error::from_str(500, format!("Failed to read cassette file: {e}")))?;

        let mut cassette: Cassette = serde_yaml::from_str(&content)
            .map_err(|e| Error::from_str(500, format!("Failed to parse cassette YAML: {e}")))?;

        cassette.path = Some(path);
        cassette.format = CassetteFormat::File;
        cassette.modified_since_load = false;

        Ok(cassette)
    }

    async fn load_from_directory(path: PathBuf) -> Result<Self, Error> {
        // Load interactions metadata from interactions.yaml
        let interactions_file = path.join("interactions.yaml");
        if !interactions_file.exists() {
            return Err(Error::from_str(
                404,
                format!("Directory cassette missing interactions.yaml: {path:?}"),
            ));
        }

        let content = std::fs::read_to_string(&interactions_file)
            .map_err(|e| Error::from_str(500, format!("Failed to read interactions.yaml: {e}")))?;

        // For directory format, we serialize a simplified structure
        #[derive(Deserialize)]
        struct DirectoryInteraction {
            request: DirectorySerializableRequest,
            response: DirectorySerializableResponse,
        }

        #[derive(Deserialize)]
        struct DirectorySerializableRequest {
            method: String,
            url: String,
            headers: std::collections::HashMap<String, Vec<String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            body_file: Option<String>,
            version: String,
        }

        #[derive(Deserialize)]
        struct DirectorySerializableResponse {
            status: u16,
            headers: std::collections::HashMap<String, Vec<String>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            body_file: Option<String>,
            version: String,
        }

        let dir_interactions: Vec<DirectoryInteraction> = serde_yaml::from_str(&content)
            .map_err(|e| Error::from_str(500, format!("Failed to parse interactions.yaml: {e}")))?;

        let bodies_dir = path.join("bodies");
        let mut interactions = Vec::new();

        for dir_interaction in dir_interactions {
            // Load request body if specified
            let (request_body, request_body_base64) =
                if let Some(ref body_file) = dir_interaction.request.body_file {
                    let body_path = bodies_dir.join(body_file);
                    let content = std::fs::read_to_string(&body_path).map_err(|e| {
                        Error::from_str(
                            500,
                            format!("Failed to read request body file {body_file}: {e}"),
                        )
                    })?;

                    // Check if this is a base64 file based on extension
                    if body_file.ends_with(".b64") {
                        (None, Some(content))
                    } else {
                        (Some(content), None)
                    }
                } else {
                    (None, None)
                };

            // Load response body if specified
            let (response_body, response_body_base64) =
                if let Some(ref body_file) = dir_interaction.response.body_file {
                    let body_path = bodies_dir.join(body_file);
                    let content = std::fs::read_to_string(&body_path).map_err(|e| {
                        Error::from_str(
                            500,
                            format!("Failed to read response body file {body_file}: {e}"),
                        )
                    })?;

                    // Check if this is a base64 file based on extension
                    if body_file.ends_with(".b64") {
                        (None, Some(content))
                    } else {
                        (Some(content), None)
                    }
                } else {
                    (None, None)
                };

            let interaction = Interaction {
                request: SerializableRequest {
                    method: dir_interaction.request.method,
                    url: dir_interaction.request.url,
                    headers: dir_interaction.request.headers,
                    body: request_body,
                    body_base64: request_body_base64,
                    version: dir_interaction.request.version,
                },
                response: SerializableResponse {
                    status: dir_interaction.response.status,
                    headers: dir_interaction.response.headers,
                    body: response_body,
                    body_base64: response_body_base64,
                    version: dir_interaction.response.version,
                },
            };

            interactions.push(interaction);
        }

        Ok(Cassette {
            interactions,
            path: Some(path),
            format: CassetteFormat::Directory,
            modified_since_load: false,
        })
    }

    pub async fn save_to_file(&self) -> Result<(), Error> {
        if let Some(path) = &self.path {
            match self.format {
                CassetteFormat::File => self.save_to_single_file(path).await,
                CassetteFormat::Directory => self.save_to_directory(path).await,
            }
        } else {
            Err(Error::from_str(400, "No path specified for cassette"))
        }
    }

    async fn save_to_single_file(&self, path: &PathBuf) -> Result<(), Error> {
        let yaml = serde_yaml::to_string(self)
            .map_err(|e| Error::from_str(500, format!("Failed to serialize cassette: {e}")))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::from_str(500, format!("Failed to create directory: {e}")))?;
        }

        std::fs::write(path, yaml)
            .map_err(|e| Error::from_str(500, format!("Failed to write cassette file: {e}")))?;

        Ok(())
    }

    async fn save_to_directory(&self, path: &PathBuf) -> Result<(), Error> {
        // Create the cassette directory and bodies subdirectory
        std::fs::create_dir_all(path).map_err(|e| {
            Error::from_str(500, format!("Failed to create cassette directory: {e}"))
        })?;

        let bodies_dir = path.join("bodies");
        std::fs::create_dir_all(&bodies_dir)
            .map_err(|e| Error::from_str(500, format!("Failed to create bodies directory: {e}")))?;

        // Create directory format structures for serialization
        use serde::Serialize;

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

        for (i, interaction) in self.interactions.iter().enumerate() {
            let interaction_num = format!("{:03}", i + 1);

            // Handle request body
            let request_body_file = if let Some(ref body) = interaction.request.body {
                if !body.is_empty() {
                    let filename = format!("req_{interaction_num}.txt");
                    let body_path = bodies_dir.join(&filename);
                    std::fs::write(&body_path, body).map_err(|e| {
                        Error::from_str(500, format!("Failed to write request body file: {e}"))
                    })?;
                    Some(filename)
                } else {
                    None
                }
            } else if let Some(ref body_base64) = interaction.request.body_base64 {
                if !body_base64.is_empty() {
                    let filename = format!("req_{interaction_num}.b64");
                    let body_path = bodies_dir.join(&filename);
                    std::fs::write(&body_path, body_base64).map_err(|e| {
                        Error::from_str(500, format!("Failed to write request body file: {e}"))
                    })?;
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
                    std::fs::write(&body_path, body).map_err(|e| {
                        Error::from_str(500, format!("Failed to write response body file: {e}"))
                    })?;
                    Some(filename)
                } else {
                    None
                }
            } else if let Some(ref body_base64) = interaction.response.body_base64 {
                if !body_base64.is_empty() {
                    let filename = format!("resp_{interaction_num}.b64");
                    let body_path = bodies_dir.join(&filename);
                    std::fs::write(&body_path, body_base64).map_err(|e| {
                        Error::from_str(500, format!("Failed to write response body file: {e}"))
                    })?;
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
            .map_err(|e| Error::from_str(500, format!("Failed to serialize interactions: {e}")))?;

        let interactions_file = path.join("interactions.yaml");
        std::fs::write(&interactions_file, interactions_yaml)
            .map_err(|e| Error::from_str(500, format!("Failed to write interactions.yaml: {e}")))?;

        Ok(())
    }

    pub fn clear(&mut self) {
        self.interactions.clear();
    }

    pub async fn record_interaction(
        &mut self,
        serializable_request: SerializableRequest,
        serializable_response: SerializableResponse,
    ) -> Result<(), Error> {
        let interaction = Interaction {
            request: serializable_request,
            response: serializable_response,
        };

        self.interactions.push(interaction);
        self.modified_since_load = true; // Mark as modified when recording new interactions
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.interactions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.interactions.is_empty()
    }
}

impl Default for Cassette {
    fn default() -> Self {
        Self::new()
    }
}
