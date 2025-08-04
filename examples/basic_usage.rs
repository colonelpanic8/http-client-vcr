fn main() {
    // For demonstration purposes, we'll show the API without actually running it
    println!("Example of using http-client-vcr:");

    println!("1. Create a VCR client with builder pattern:");
    println!(
        r#"
let vcr_client = VcrClient::builder()
    .inner_client(inner_client)
    .cassette_path("fixtures/my_test.yaml")
    .mode(VcrMode::Once)
    .build()
    .await?;
"#
    );

    println!("2. Use it like any HttpClient:");
    println!(
        r#"
let request = Request::new(Method::Get, Url::parse("https://httpbin.org/get")?);
let response = vcr_client.send(request).await?;
println!("Status: {{}}", response.status());
"#
    );

    println!("3. Save cassette after use:");
    println!(
        r#"
vcr_client.save_cassette().await?;
"#
    );

    println!("4. VCR Modes:");
    println!("   - VcrMode::Record: Always record new interactions");
    println!("   - VcrMode::Replay: Only replay from cassette, fail if not found");
    println!("   - VcrMode::Once: Record once, then replay");
    println!("   - VcrMode::None: Pass through without recording");
}
