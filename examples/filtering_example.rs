fn main() {
    println!("Example of using http-client-vcr with filtering:");

    println!("1. Remove sensitive headers:");
    println!(
        r#"
let header_filter = HeaderFilter::new()
    .remove_auth_headers()
    .remove_header("X-Custom-Secret")
    .replace_header("User-Id", "FILTERED");
"#
    );

    println!("2. Filter sensitive data from JSON bodies:");
    println!(
        r#"
let body_filter = BodyFilter::new()
    .remove_common_sensitive_keys()
    .remove_json_key("credit_card")
    .replace_regex(r"\d{{4}}-\d{{4}}-\d{{4}}-\d{{4}}", "XXXX-XXXX-XXXX-XXXX")?;
"#
    );

    println!("3. Remove sensitive query parameters:");
    println!(
        r#"
let url_filter = UrlFilter::new()
    .remove_common_sensitive_params()
    .remove_query_param("secret")
    .replace_query_param("user_id", "FILTERED");
"#
    );

    println!("4. Chain filters together:");
    println!(
        r#"
let filter_chain = FilterChain::new()
    .add_filter(Box::new(header_filter))
    .add_filter(Box::new(body_filter))
    .add_filter(Box::new(url_filter));
"#
    );

    println!("5. Use with VcrClient:");
    println!(
        r#"
let vcr_client = VcrClient::builder()
    .inner_client(inner_client)
    .cassette_path("fixtures/filtered_test.yaml")
    .mode(VcrMode::Once)
    .filter_chain(filter_chain)
    .build()
    .await?;
"#
    );

    println!("6. Or add filters individually:");
    println!(
        r#"
let vcr_client = VcrClient::builder()
    .inner_client(inner_client)
    .cassette_path("fixtures/filtered_test.yaml")
    .mode(VcrMode::Once)
    .add_filter(Box::new(HeaderFilter::new().remove_auth_headers()))
    .add_filter(Box::new(BodyFilter::new().remove_common_sensitive_keys()))
    .build()
    .await?;
"#
    );

    println!("7. Custom filters:");
    println!("use http_client_vcr::CustomFilter;");
    println!();
    println!("let custom_filter = CustomFilter::new(|req, resp| {{");
    println!("    // Remove any header containing \"secret\"");
    println!("    req.headers.retain(|key, _| !key.to_lowercase().contains(\"secret\"));");
    println!("    ");
    println!("    // Replace response body if it contains errors");
    println!("    if let Some(body) = &mut resp.body {{");
    println!("        if body.contains(\"error\") {{");
    println!("            *body = r#\"{{\"error\": \"FILTERED\"}}\"#.to_string();");
    println!("        }}");
    println!("    }}");
    println!("}});");
    println!();
    println!("let vcr_client = VcrClient::builder()");
    println!("    .inner_client(inner_client)");
    println!("    .add_filter(Box::new(custom_filter))");
    println!("    .build()");
    println!("    .await?;");

    println!("\nIMPORTANT: How filtering works:");
    println!("- During recording: Real requests are made with original sensitive data");
    println!("- Your application receives the real, unfiltered responses");
    println!("- Only the stored cassette data gets filtered (removing sensitive info)");
    println!("- This keeps cassettes safe for version control while preserving functionality");
}
