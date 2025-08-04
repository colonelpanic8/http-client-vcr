use crate::serializable::{SerializableRequest, SerializableResponse};
use http_client::Error;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    pub request: SerializableRequest,
    pub response: SerializableResponse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Cassette {
    pub interactions: Vec<Interaction>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Cassette {
    pub fn new() -> Self {
        Self {
            interactions: Vec::new(),
            path: None,
        }
    }

    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    pub async fn load_from_file(path: PathBuf) -> Result<Self, Error> {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| Error::from_str(500, format!("Failed to read cassette file: {e}")))?;

        let mut cassette: Cassette = serde_yaml::from_str(&content)
            .map_err(|e| Error::from_str(500, format!("Failed to parse cassette YAML: {e}")))?;

        cassette.path = Some(path);

        Ok(cassette)
    }

    pub async fn save_to_file(&self) -> Result<(), Error> {
        if let Some(path) = &self.path {
            let yaml = serde_yaml::to_string(self)
                .map_err(|e| Error::from_str(500, format!("Failed to serialize cassette: {e}")))?;

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::from_str(500, format!("Failed to create directory: {e}"))
                })?;
            }

            std::fs::write(path, yaml)
                .map_err(|e| Error::from_str(500, format!("Failed to write cassette file: {e}")))?;

            Ok(())
        } else {
            Err(Error::from_str(400, "No path specified for cassette"))
        }
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
