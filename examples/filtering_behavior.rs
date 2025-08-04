fn main() {
    println!("Filtering Behavior Example");
    println!("==========================");
    println!();

    println!("Consider this scenario:");
    println!("1. Your app makes a request with Authorization: 'Bearer secret-token'");
    println!("2. The API returns: {{\"user_id\": 12345, \"balance\": 1000.50}}");
    println!();

    println!("With VCR filtering:");
    println!();

    println!("RECORDING MODE:");
    println!("├─ Real request sent → Authorization: 'Bearer secret-token'");
    println!("├─ Real response received ← {{\"user_id\": 12345, \"balance\": 1000.50}}");
    println!("├─ Your app gets the real response (unfiltered)");
    println!("└─ Cassette stores filtered version:");
    println!("   Request: Authorization: 'FILTERED'");
    println!("   Response: {{\"user_id\": \"FILTERED\", \"balance\": 1000.50}}");
    println!();

    println!("REPLAY MODE:");
    println!("├─ No real HTTP request made");
    println!("├─ Response from cassette: {{\"user_id\": \"FILTERED\", \"balance\": 1000.50}}");
    println!("└─ Your app gets the stored (filtered) response");
    println!();

    println!("Configuration example:");
    println!("let vcr_client = VcrClient::builder()");
    println!("    .inner_client(client)");
    println!("    .cassette_path(\"test.yaml\")");
    println!("    .add_filter(Box::new(");
    println!("        HeaderFilter::new().remove_header(\"Authorization\")");
    println!("    ))");
    println!("    .add_filter(Box::new(");
    println!("        BodyFilter::new().replace_json_key(\"user_id\", \"FILTERED\")");
    println!("    ))");
    println!("    .build()");
    println!("    .await?;");
    println!();

    println!("Benefits:");
    println!("✓ APIs work normally during recording (real credentials used)");
    println!("✓ Tests are deterministic during replay (no network calls)");
    println!("✓ Cassettes are safe to commit (no sensitive data stored)");
    println!("✓ Flexible filtering for different sensitivity levels");
}
