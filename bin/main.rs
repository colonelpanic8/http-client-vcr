use clap::{Arg, Command};
use http_client_vcr::{Cassette, CassetteFormat, Interaction};
use serde_json::{json, Value};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let matches = Command::new("vcr-inspect")
        .version("0.2.0")
        .about("Inspect VCR cassettes")
        .subcommand(
            Command::new("list")
                .about("List all requests in a cassette")
                .arg(
                    Arg::new("cassette")
                        .help("Path to the cassette file or directory")
                        .required(true)
                        .index(1),
                ),
        )
        .subcommand(
            Command::new("field")
                .about("Extract specific fields from cassette interactions")
                .arg(
                    Arg::new("cassette")
                        .help("Path to the cassette file or directory")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::new("field")
                        .help("Field path to extract (e.g., 'request.method', 'response.status')")
                        .required(true)
                        .index(2),
                )
                .arg(
                    Arg::new("interaction")
                        .help("Interaction index (0-based). If not specified, shows all interactions")
                        .long("interaction")
                        .short('i')
                        .value_parser(clap::value_parser!(usize)),
                ),
        )
        .subcommand(
            Command::new("convert")
                .about("Convert cassette between file and directory formats")
                .arg(
                    Arg::new("source")
                        .help("Path to the source cassette file or directory")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::new("destination")
                        .help("Path to the destination cassette file or directory")
                        .required(true)
                        .index(2),
                )
                .arg(
                    Arg::new("format")
                        .help("Output format: 'file' or 'directory'")
                        .required(true)
                        .long("format")
                        .short('f')
                        .value_parser(["file", "directory"]),
                ),
        )
        .subcommand(
            Command::new("fields")
                .about("List all available field paths in a cassette")
                .arg(
                    Arg::new("cassette")
                        .help("Path to the cassette file or directory")
                        .required(true)
                        .index(1),
                )
                .arg(
                    Arg::new("interaction")
                        .help("Interaction index (0-based). If not specified, shows fields from first interaction")
                        .long("interaction")
                        .short('i')
                        .value_parser(clap::value_parser!(usize)),
                ),
        )
        .get_matches();

    let result = match matches.subcommand() {
        Some(("list", sub_matches)) => {
            let cassette_path = sub_matches.get_one::<String>("cassette").unwrap();
            list_requests(cassette_path).await
        }
        Some(("field", sub_matches)) => {
            let cassette_path = sub_matches.get_one::<String>("cassette").unwrap();
            let field_path = sub_matches.get_one::<String>("field").unwrap();
            let interaction_idx = sub_matches.get_one::<usize>("interaction").copied();
            extract_field(cassette_path, field_path, interaction_idx).await
        }
        Some(("convert", sub_matches)) => {
            let source_path = sub_matches.get_one::<String>("source").unwrap();
            let destination_path = sub_matches.get_one::<String>("destination").unwrap();
            let format = sub_matches.get_one::<String>("format").unwrap();
            convert_cassette(source_path, destination_path, format).await
        }
        Some(("fields", sub_matches)) => {
            let cassette_path = sub_matches.get_one::<String>("cassette").unwrap();
            let interaction_idx = sub_matches.get_one::<usize>("interaction").copied();
            list_fields(cassette_path, interaction_idx).await
        }
        _ => {
            eprintln!("No subcommand provided. Use --help for usage information.");
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

async fn list_requests(cassette_path: &str) -> Result<(), String> {
    let path = PathBuf::from(cassette_path);
    let cassette = Cassette::load_from_file(path)
        .await
        .map_err(|e| format!("Failed to load cassette: {e}"))?;

    let mut requests = Vec::new();
    for (index, interaction) in cassette.interactions.iter().enumerate() {
        requests.push(json!({
            "index": index,
            "method": interaction.request.method,
            "url": interaction.request.url,
            "status": interaction.response.status
        }));
    }

    let output = json!({
        "total_interactions": cassette.interactions.len(),
        "requests": requests
    });

    println!("{}", serde_json::to_string(&output).unwrap());
    Ok(())
}

async fn extract_field(
    cassette_path: &str,
    field_path: &str,
    interaction_idx: Option<usize>,
) -> Result<(), String> {
    let path = PathBuf::from(cassette_path);
    let cassette = Cassette::load_from_file(path)
        .await
        .map_err(|e| format!("Failed to load cassette: {e}"))?;

    if let Some(idx) = interaction_idx {
        if idx >= cassette.interactions.len() {
            return Err(format!(
                "Interaction index {} out of bounds (total: {})",
                idx,
                cassette.interactions.len()
            ));
        }
        let interaction = &cassette.interactions[idx];
        let value = extract_field_from_interaction(interaction, field_path)?;
        print_value(&value);
    } else {
        let mut results = Vec::new();
        for (index, interaction) in cassette.interactions.iter().enumerate() {
            match extract_field_from_interaction(interaction, field_path) {
                Ok(value) => results.push(json!({
                    "index": index,
                    "value": value
                })),
                Err(_) => results.push(json!({
                    "index": index,
                    "value": null
                })),
            }
        }
        println!("{}", serde_json::to_string(&results).unwrap());
    }

    Ok(())
}

fn extract_field_from_interaction(
    interaction: &Interaction,
    field_path: &str,
) -> Result<Value, String> {
    let interaction_json = serde_json::to_value(interaction)
        .map_err(|e| format!("Failed to serialize interaction: {e}"))?;

    extract_nested_field(&interaction_json, field_path)
}

fn extract_nested_field(value: &Value, field_path: &str) -> Result<Value, String> {
    let parts = parse_field_path(field_path);
    let mut current = value;

    for part in parts {
        match part {
            FieldPathPart::Key(key) => match current {
                Value::Object(map) => {
                    current = map
                        .get(&key)
                        .ok_or_else(|| format!("Field '{key}' not found in object"))?;
                }
                _ => {
                    return Err(format!("Cannot access field '{key}' on non-object value"));
                }
            },
            FieldPathPart::Index(index) => match current {
                Value::Array(arr) => {
                    current = arr.get(index).ok_or_else(|| {
                        format!(
                            "Array index {} out of bounds (length: {})",
                            index,
                            arr.len()
                        )
                    })?;
                }
                _ => {
                    return Err(format!("Cannot access index {index} on non-array value"));
                }
            },
        }
    }

    Ok(current.clone())
}

#[derive(Debug)]
enum FieldPathPart {
    Key(String),
    Index(usize),
}

fn parse_field_path(field_path: &str) -> Vec<FieldPathPart> {
    let mut parts = Vec::new();
    let mut current_part = String::new();
    let mut chars = field_path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current_part.is_empty() {
                    parts.push(FieldPathPart::Key(current_part.clone()));
                    current_part.clear();
                }
            }
            '[' => {
                if !current_part.is_empty() {
                    parts.push(FieldPathPart::Key(current_part.clone()));
                    current_part.clear();
                }

                // Parse array index
                let mut index_str = String::new();
                for index_ch in chars.by_ref() {
                    if index_ch == ']' {
                        break;
                    }
                    index_str.push(index_ch);
                }

                if let Ok(index) = index_str.parse::<usize>() {
                    parts.push(FieldPathPart::Index(index));
                }
            }
            _ => {
                current_part.push(ch);
            }
        }
    }

    if !current_part.is_empty() {
        parts.push(FieldPathPart::Key(current_part));
    }

    parts
}

fn print_value(value: &Value) {
    match value {
        Value::String(s) => {
            // Print raw string content without JSON escaping
            print!("{s}");
        }
        _ => {
            // For non-string values, use JSON serialization
            print!("{}", serde_json::to_string(value).unwrap());
        }
    }
}

async fn convert_cassette(
    source_path: &str,
    destination_path: &str,
    format: &str,
) -> Result<(), String> {
    let source = PathBuf::from(source_path);
    let destination = PathBuf::from(destination_path);

    let target_format = match format {
        "file" => CassetteFormat::File,
        "directory" => CassetteFormat::Directory,
        _ => {
            return Err(format!(
                "Invalid format '{format}'. Must be 'file' or 'directory'"
            ))
        }
    };

    let mut cassette = Cassette::load_from_file(source)
        .await
        .map_err(|e| format!("Failed to load source cassette: {e}"))?;

    cassette = cassette.with_path(destination).with_format(target_format);

    cassette
        .save_to_file()
        .await
        .map_err(|e| format!("Failed to save converted cassette: {e}"))?;

    let result = json!({
        "success": true,
        "source_path": source_path,
        "destination_path": destination_path,
        "format": format,
        "interactions_converted": cassette.interactions.len()
    });

    println!("{}", serde_json::to_string(&result).unwrap());
    Ok(())
}

async fn list_fields(cassette_path: &str, interaction_idx: Option<usize>) -> Result<(), String> {
    let path = PathBuf::from(cassette_path);
    let cassette = Cassette::load_from_file(path)
        .await
        .map_err(|e| format!("Failed to load cassette: {e}"))?;

    if cassette.interactions.is_empty() {
        return Err("Cassette contains no interactions".to_string());
    }

    let idx = interaction_idx.unwrap_or(0);
    if idx >= cassette.interactions.len() {
        return Err(format!(
            "Interaction index {} out of bounds (total: {})",
            idx,
            cassette.interactions.len()
        ));
    }

    let interaction = &cassette.interactions[idx];
    let interaction_json = serde_json::to_value(interaction)
        .map_err(|e| format!("Failed to serialize interaction: {e}"))?;

    let mut field_paths = Vec::new();
    collect_field_paths(&interaction_json, "", &mut field_paths);

    let result = json!({
        "interaction_index": idx,
        "total_interactions": cassette.interactions.len(),
        "field_paths": field_paths
    });

    println!("{}", serde_json::to_string(&result).unwrap());
    Ok(())
}

fn collect_field_paths(value: &Value, current_path: &str, paths: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                let new_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };

                paths.push(new_path.clone());
                collect_field_paths(val, &new_path, paths);
            }
        }
        Value::Array(arr) => {
            for (index, val) in arr.iter().enumerate() {
                let new_path = format!("{current_path}[{index}]");
                paths.push(new_path.clone());
                collect_field_paths(val, &new_path, paths);
            }
        }
        _ => {
            // Leaf values are already added by their parent
        }
    }
}
