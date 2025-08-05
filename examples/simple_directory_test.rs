use http_client_vcr::{Cassette, CassetteFormat, SerializableRequest, SerializableResponse};
use std::collections::HashMap;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Testing directory-based cassette format...");

    // Create test directory
    let test_dir = PathBuf::from("test_directory_cassette");

    // Clean up any existing test directory
    if test_dir.exists() {
        std::fs::remove_dir_all(&test_dir)?;
    }

    // Create a cassette with directory format
    let mut cassette = Cassette::new()
        .with_path(test_dir.clone())
        .with_format(CassetteFormat::Directory);

    // Create test interactions
    let request1 = SerializableRequest {
        method: "GET".to_string(),
        url: "https://example.com/api/test".to_string(),
        headers: {
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                vec!["application/json".to_string()],
            );
            headers
        },
        body: Some("test request body".to_string()),
        body_base64: None,
        version: "HTTP/1.1".to_string(),
    };

    let response1 = SerializableResponse {
        status: 200,
        headers: {
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                vec!["application/json".to_string()],
            );
            headers
        },
        body: Some(r#"{"message": "Hello, World!", "status": "success"}"#.to_string()),
        body_base64: None,
        version: "HTTP/1.1".to_string(),
    };

    let request2 = SerializableRequest {
        method: "POST".to_string(),
        url: "https://example.com/api/data".to_string(),
        headers: {
            let mut headers = HashMap::new();
            headers.insert("content-type".to_string(), vec!["text/html".to_string()]);
            headers
        },
        body: None,
        body_base64: Some("VGhpcyBpcyBhIGJhc2U2NCBlbmNvZGVkIGJvZHk=".to_string()), // "This is a base64 encoded body"
        version: "HTTP/1.1".to_string(),
    };

    let response2 = SerializableResponse {
        status: 201,
        headers: {
            let mut headers = HashMap::new();
            headers.insert("content-type".to_string(), vec!["text/html".to_string()]);
            headers
        },
        body: Some("<html><body><h1>Created Successfully</h1></body></html>".to_string()),
        body_base64: None,
        version: "HTTP/1.1".to_string(),
    };

    // Record interactions
    cassette.record_interaction(request1, response1).await?;
    cassette.record_interaction(request2, response2).await?;

    println!("✅ Created cassette with {} interactions", cassette.len());

    // Save the cassette in directory format
    cassette.save_to_file().await?;
    println!("✅ Saved cassette to directory format");

    // Display the directory structure
    println!("\n📁 Directory structure:");
    print_directory_structure(&test_dir, 0)?;

    // Load the cassette back from directory format
    println!("\n🔄 Loading cassette from directory format...");
    let loaded_cassette = Cassette::load_from_file(test_dir.clone()).await?;

    println!(
        "✅ Loaded cassette with {} interactions",
        loaded_cassette.len()
    );

    // Verify the loaded data
    for (i, interaction) in loaded_cassette.interactions.iter().enumerate() {
        println!("\n📝 Interaction {}:", i + 1);
        println!("  Method: {}", interaction.request.method);
        println!("  URL: {}", interaction.request.url);
        println!("  Request body: {:?}", interaction.request.body);
        println!(
            "  Request body_base64: {:?}",
            interaction.request.body_base64
        );
        println!("  Response status: {}", interaction.response.status);
        println!(
            "  Response body length: {}",
            interaction
                .response
                .body
                .as_ref()
                .map(|b| b.len())
                .unwrap_or(0)
        );
    }

    println!("\n🎉 Directory format test completed successfully!");

    // Clean up
    std::fs::remove_dir_all(&test_dir)?;
    println!("🧹 Cleaned up test directory");

    Ok(())
}

fn print_directory_structure(dir: &PathBuf, indent: usize) -> Result<(), std::io::Error> {
    let indent_str = "  ".repeat(indent);

    if dir.is_dir() {
        println!(
            "{}📁 {}/",
            indent_str,
            dir.file_name().unwrap_or_default().to_string_lossy()
        );

        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                print_directory_structure(&path, indent + 1)?;
            } else {
                let file_name = path.file_name().unwrap().to_string_lossy();
                let size = std::fs::metadata(&path)?.len();
                println!(
                    "{}📄 {} ({} bytes)",
                    "  ".repeat(indent + 1),
                    file_name,
                    size
                );

                // Show content preview for small files
                if size < 200 {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let preview = if content.len() > 100 {
                            format!("{}...", &content[..100])
                        } else {
                            content
                        };
                        println!(
                            "{}    \"{}\"",
                            "  ".repeat(indent + 1),
                            preview.replace('\n', "\\n")
                        );
                    }
                }
            }
        }
    }

    Ok(())
}
