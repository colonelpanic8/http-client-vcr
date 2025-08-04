use std::collections::HashMap;

/// Parse URL-encoded form data into key-value pairs
pub fn parse_form_data(data: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    for pair in data.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            // URL decode the key and value
            let decoded_key = urlencoding::decode(key).unwrap_or_else(|_| key.into());
            let decoded_value = urlencoding::decode(value).unwrap_or_else(|_| value.into());
            params.insert(decoded_key.to_string(), decoded_value.to_string());
        }
    }

    params
}

/// Encode form data back to URL-encoded string
pub fn encode_form_data(params: &HashMap<String, String>) -> String {
    params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Detect potential credential fields in form data
pub fn find_credential_fields(params: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut credentials = Vec::new();

    // Common field names that might contain credentials
    let credential_patterns = [
        // Username patterns
        "username",
        "user",
        "login",
        "email",
        "username_or_email",
        "user_name",
        // Password patterns
        "password",
        "pass",
        "passwd",
        "pwd",
        "secret",
        // Token/CSRF patterns
        "_token",
        // Session patterns
        "session",
        "sessionid",
        "sid",
        "auth",
        "authorization",
        // API key patterns
        "api_key",
        "apikey",
        "key",
        "client_secret",
        "access_token",
        "refresh_token",
    ];

    for (key, value) in params {
        let key_lower = key.to_lowercase();

        // Check if the key matches any credential pattern
        for pattern in &credential_patterns {
            if key_lower.contains(pattern) {
                credentials.push((key.clone(), value.clone()));
                break;
            }
        }

        // Also check for suspicious values (long alphanumeric strings that might be tokens)
        if value.len() > 10 && value.chars().all(|c| c.is_alphanumeric()) {
            // This might be a token or hash
            credentials.push((key.clone(), value.clone()));
        }
    }

    credentials
}

/// Filter sensitive form data by replacing credential values
pub fn filter_form_data(data: &str, replacement_pattern: &str) -> String {
    let mut params = parse_form_data(data);
    let credentials = find_credential_fields(&params);

    // Replace sensitive values
    for (key, _) in credentials {
        if let Some(value) = params.get_mut(&key) {
            *value = format!("{replacement_pattern}_{}", key.to_uppercase());
        }
    }

    encode_form_data(&params)
}

/// Analyze form data and return a report of what was found
pub fn analyze_form_data(data: &str) -> FormDataAnalysis {
    let params = parse_form_data(data);
    let credentials = find_credential_fields(&params);

    FormDataAnalysis {
        total_fields: params.len(),
        credential_fields: credentials,
        all_fields: params,
    }
}

#[derive(Debug)]
pub struct FormDataAnalysis {
    pub total_fields: usize,
    pub credential_fields: Vec<(String, String)>,
    pub all_fields: HashMap<String, String>,
}

impl FormDataAnalysis {
    /// Print a summary of the analysis
    pub fn print_summary(&self) {
        println!("Form Data Analysis:");
        println!("  Total fields: {}", self.total_fields);
        println!(
            "  Credential fields found: {}",
            self.credential_fields.len()
        );

        if !self.credential_fields.is_empty() {
            println!("  Sensitive fields:");
            for (key, value) in &self.credential_fields {
                let preview = if value.len() > 20 {
                    format!("{}...", &value[..20])
                } else {
                    value.clone()
                };
                println!("    {key}: {preview}");
            }
        }

        println!("  All fields:");
        for (key, value) in &self.all_fields {
            let preview = if value.len() > 50 {
                format!("{}...", &value[..50])
            } else {
                value.clone()
            };
            println!("    {key}: {preview}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_form_data() {
        let data = "username=testuser&password=secret123&csrf_token=abc123";
        let params = parse_form_data(data);

        assert_eq!(params.get("username"), Some(&"testuser".to_string()));
        assert_eq!(params.get("password"), Some(&"secret123".to_string()));
        assert_eq!(params.get("csrf_token"), Some(&"abc123".to_string()));
    }

    #[test]
    fn test_find_credential_fields() {
        let mut params = HashMap::new();
        params.insert("username".to_string(), "testuser".to_string());
        params.insert("password".to_string(), "secret123".to_string());
        params.insert("normal_field".to_string(), "value".to_string());

        let credentials = find_credential_fields(&params);
        assert_eq!(credentials.len(), 2);

        let keys: Vec<&String> = credentials.iter().map(|(k, _)| k).collect();
        assert!(keys.contains(&&"username".to_string()));
        assert!(keys.contains(&&"password".to_string()));
    }

    #[test]
    fn test_filter_form_data() {
        let data = "username=testuser&password=secret123&normal=value";
        let filtered = filter_form_data(data, "[FILTERED]");

        // The brackets get URL-encoded, so we need to check for the encoded version
        assert!(filtered.contains("%5BFILTERED%5D_USERNAME"));
        assert!(filtered.contains("%5BFILTERED%5D_PASSWORD"));
        assert!(filtered.contains("normal=value"));
    }
}
