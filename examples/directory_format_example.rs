use http_client::HttpClient;
use http_client_vcr::{CassetteFormat, NoOpClient, VcrClient, VcrMode};
use http_types::{Method, Url};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a directory-based cassette
    let cassette_dir = PathBuf::from("example_directory_cassette");

    // Clean up any existing cassette for this example
    if cassette_dir.exists() {
        std::fs::remove_dir_all(&cassette_dir)?;
    }

    // Create a VCR client in Record mode using directory format
    let vcr_client = VcrClient::builder(&cassette_dir)
        .inner_client(Box::new(NoOpClient::new())) // Use NoOpClient for demonstration
        .mode(VcrMode::Record)
        .format(CassetteFormat::Directory) // Use directory format
        .build()
        .await?;

    println!("🎬 Recording HTTP requests to directory format...");

    // Make some HTTP requests
    let mut req1 = http_types::Request::new(Method::Get, Url::parse("https://httpbin.org/get")?);
    req1.set_body("test request body");
    let _response1 = vcr_client.send(req1).await?;
    println!("✅ Recorded GET request");

    let req2 = http_types::Request::new(Method::Post, Url::parse("https://httpbin.org/post")?);
    let _response2 = vcr_client.send(req2).await?;
    println!("✅ Recorded POST request");

    // Drop the client to save the cassette
    drop(vcr_client);

    println!("\n📁 Directory structure created:");
    if cassette_dir.exists() {
        print_directory_structure(&cassette_dir, 0)?;
    }

    println!("\n🔄 Now replaying from directory format...");

    // Create a new VCR client in Replay mode
    let replay_client = VcrClient::builder(&cassette_dir)
        .inner_client(Box::new(NoOpClient::new())) // Use NoOpClient for demonstration
        .mode(VcrMode::Replay)
        .build()
        .await?;

    // Replay the requests
    let req1 = http_types::Request::new(Method::Get, Url::parse("https://httpbin.org/get")?);
    let response1 = replay_client.send(req1).await?;
    println!("✅ Replayed GET request - Status: {}", response1.status());

    let req2 = http_types::Request::new(Method::Post, Url::parse("https://httpbin.org/post")?);
    let response2 = replay_client.send(req2).await?;
    println!("✅ Replayed POST request - Status: {}", response2.status());

    println!("\n🎉 Directory format example completed successfully!");
    println!(
        "💡 You can now inspect the individual body files in {}/bodies/",
        cassette_dir.display()
    );

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
                println!("{indent_str}📄 {file_name} ({size} bytes)");
            }
        }
    }

    Ok(())
}
