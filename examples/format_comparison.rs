use http_client_vcr::{Cassette, CassetteFormat, SerializableRequest, SerializableResponse};
use std::collections::HashMap;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔄 Comparing file vs directory cassette formats...\n");

    // Create some test data
    let test_request = SerializableRequest {
        method: "POST".to_string(),
        url: "https://api.example.com/users".to_string(),
        headers: {
            let mut headers = HashMap::new();
            headers.insert("content-type".to_string(), vec!["application/json".to_string()]);
            headers.insert("authorization".to_string(), vec!["Bearer token123".to_string()]);
            headers
        },
        body: Some(r#"{"name": "John Doe", "email": "john@example.com", "profile": "A very long bio that contains lots of information about the user, including their interests, background, and other detailed information that would make a large body payload."}"#.to_string()),
        body_base64: None,
        version: "HTTP/1.1".to_string(),
    };

    let test_response = SerializableResponse {
        status: 201,
        headers: {
            let mut headers = HashMap::new();
            headers.insert("content-type".to_string(), vec!["application/json".to_string()]);
            headers.insert("location".to_string(), vec!["/users/123".to_string()]);
            headers
        },
        body: Some(r#"{"id": 123, "name": "John Doe", "email": "john@example.com", "created_at": "2024-01-01T00:00:00Z", "profile": "A very long bio that contains lots of information about the user, including their interests, background, and other detailed information that would make a large body payload.", "preferences": {"theme": "dark", "notifications": true, "language": "en"}}"#.to_string()),
        body_base64: None,
        version: "HTTP/1.1".to_string(),
    };

    // Test 1: File format
    println!("📄 Testing traditional file format...");
    let file_path = PathBuf::from("test_file_cassette.yaml");

    // Clean up any existing files
    if file_path.exists() {
        std::fs::remove_file(&file_path)?;
    }

    let mut file_cassette = Cassette::new()
        .with_path(file_path.clone())
        .with_format(CassetteFormat::File);

    file_cassette
        .record_interaction(test_request.clone(), test_response.clone())
        .await?;
    file_cassette.save_to_file().await?;

    let file_size = std::fs::metadata(&file_path)?.len();
    println!(
        "  ✅ Saved to single file: {} ({} bytes)",
        file_path.display(),
        file_size
    );

    // Test 2: Directory format
    println!("\n📁 Testing directory format...");
    let dir_path = PathBuf::from("test_directory_cassette");

    // Clean up any existing directory
    if dir_path.exists() {
        std::fs::remove_dir_all(&dir_path)?;
    }

    let mut dir_cassette = Cassette::new()
        .with_path(dir_path.clone())
        .with_format(CassetteFormat::Directory);

    dir_cassette
        .record_interaction(test_request.clone(), test_response.clone())
        .await?;
    dir_cassette.save_to_file().await?;

    // Calculate total directory size
    let dir_size = calculate_directory_size(&dir_path)?;
    println!(
        "  ✅ Saved to directory: {} ({} bytes total)",
        dir_path.display(),
        dir_size
    );

    // Show directory structure
    println!("\n📁 Directory structure:");
    print_directory_structure(&dir_path, 1)?;

    // Test loading both formats
    println!("\n🔄 Testing loading...");

    let loaded_file_cassette = Cassette::load_from_file(file_path.clone()).await?;
    println!(
        "  ✅ Loaded file format: {} interactions",
        loaded_file_cassette.len()
    );

    let loaded_dir_cassette = Cassette::load_from_file(dir_path.clone()).await?;
    println!(
        "  ✅ Loaded directory format: {} interactions",
        loaded_dir_cassette.len()
    );

    // Verify data integrity
    println!("\n🔍 Verifying data integrity...");
    let file_interaction = &loaded_file_cassette.interactions[0];
    let dir_interaction = &loaded_dir_cassette.interactions[0];

    let matches = file_interaction.request.method == dir_interaction.request.method
        && file_interaction.request.url == dir_interaction.request.url
        && file_interaction.request.body == dir_interaction.request.body
        && file_interaction.response.status == dir_interaction.response.status
        && file_interaction.response.body == dir_interaction.response.body;

    if matches {
        println!("  ✅ Data integrity verified - both formats contain identical data");
    } else {
        println!("  ❌ Data mismatch between formats!");
    }

    // Format comparison
    println!("\n📊 Format Comparison:");
    println!("  File format:");
    println!("    - Single YAML file");
    println!("    - Size: {file_size} bytes");
    println!("    - Easy to version control as single file");
    println!("    - Bodies embedded in YAML (may need base64 encoding)");

    println!("  Directory format:");
    println!("    - Structured directory with separate body files");
    println!("    - Size: {dir_size} bytes total");
    println!("    - Bodies stored as separate files (easy to inspect/edit)");
    println!("    - Better for large payloads and binary content");
    println!("    - Interaction metadata in interactions.yaml");

    // Clean up
    std::fs::remove_file(&file_path)?;
    std::fs::remove_dir_all(&dir_path)?;
    println!("\n🧹 Cleaned up test files");

    println!("\n🎉 Format comparison completed!");
    println!("💡 Use CassetteFormat::Directory for large payloads or when you need to inspect/edit bodies");
    println!("💡 Use CassetteFormat::File (default) for simple cases and easy version control");

    Ok(())
}

fn calculate_directory_size(dir: &PathBuf) -> Result<u64, std::io::Error> {
    let mut total_size = 0;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            total_size += calculate_directory_size(&path)?;
        } else {
            total_size += std::fs::metadata(&path)?.len();
        }
    }

    Ok(total_size)
}

fn print_directory_structure(dir: &PathBuf, indent: usize) -> Result<(), std::io::Error> {
    let indent_str = "  ".repeat(indent);

    if dir.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                println!(
                    "{}📁 {}/",
                    indent_str,
                    path.file_name().unwrap().to_string_lossy()
                );
                print_directory_structure(&path, indent + 1)?;
            } else {
                let file_name = path.file_name().unwrap().to_string_lossy();
                let size = std::fs::metadata(&path)?.len();
                println!("{indent_str}📄 {file_name} ({size} bytes)");
            }
        }
    }

    Ok(())
}
