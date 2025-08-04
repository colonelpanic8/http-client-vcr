fn main() {
    println!("NoOpClient Example");
    println!("==================");
    println!();

    println!("The NoOpClient is designed to ensure no real HTTP requests are made during testing.");
    println!("It's particularly useful when you want to be absolutely certain that your tests");
    println!("are only using recorded interactions from cassettes.");
    println!();

    println!("Basic usage:");
    println!("```rust");
    println!("use http_client_vcr::{{VcrClient, VcrMode, NoOpClient}};");
    println!();
    println!("// This guarantees no real HTTP requests will be made");
    println!("let vcr_client = VcrClient::builder()");
    println!("    .inner_client(Box::new(NoOpClient::new()))");
    println!("    .cassette_path(\"tests/fixtures/api_test.yaml\")");
    println!("    .mode(VcrMode::Replay)  // Only replay from cassette");
    println!("    .build()");
    println!("    .await?;");
    println!();
    println!("// This will work if the request exists in the cassette");
    println!("let response = vcr_client.send(request).await?;");
    println!("```");
    println!();

    println!("Two variants available:");
    println!();

    println!("1. **NoOpClient::new()** - Returns an error if a request is attempted:");
    println!("   - Useful for production-like test environments");
    println!("   - Provides clear error messages about VCR misconfiguration");
    println!("   - Allows tests to handle the error gracefully");
    println!();

    println!("2. **NoOpClient::panicking()** - Panics if a request is attempted:");
    println!("   - Useful during development");
    println!("   - Provides immediate feedback with stack traces");
    println!("   - Helps identify exactly where unexpected requests originate");
    println!();

    println!("Custom error messages:");
    println!("```rust");
    println!("// Custom error message");
    println!("let client = NoOpClient::with_message(");
    println!("    \"Test configuration error: Real HTTP requests detected\"");
    println!(");");
    println!();
    println!("// Custom panic message");
    println!("let client = PanickingNoOpClient::with_message(");
    println!("    \"DEVELOPMENT ERROR: Unexpected HTTP request in test!\"");
    println!(");");
    println!("```");
    println!();

    println!("How it works:");
    println!("1. VCR first checks if the request exists in the cassette");
    println!("2. If found → returns the recorded response (NoOpClient never called)");
    println!("3. If not found → VCR returns 404 error (NoOpClient never called)");
    println!("4. NoOpClient only gets called if there's a bug in VCR or misconfiguration");
    println!();

    println!("Benefits:");
    println!("✓ Absolute guarantee that no network requests are made");
    println!("✓ Clear error messages when something goes wrong");
    println!("✓ Helps catch VCR configuration issues early");
    println!("✓ Perfect for CI/CD environments where network access is restricted");
    println!("✓ Useful during development to ensure tests are deterministic");
}
