# Directory-Based Cassette Format

The HTTP Client VCR library now supports a new directory-based cassette format alongside the traditional single-file YAML format. This new format stores request/response bodies as separate files, making it easier to inspect and edit large payloads.

## Format Comparison

### Traditional File Format (Default)
- **Structure**: Single YAML file containing all interaction data
- **Bodies**: Embedded in YAML (may require base64 encoding for binary/complex content)
- **Best for**: Simple cases, easy version control, small payloads
- **File**: `cassette.yaml`

### Directory Format
- **Structure**: Directory with metadata file and separate body files
- **Bodies**: Stored as individual files in `bodies/` subdirectory
- **Best for**: Large payloads, binary content, easier inspection/editing of responses
- **Structure**:
  ```
  my_cassette/
  ├── interactions.yaml     # Metadata (headers, URLs, status codes)
  └── bodies/
      ├── req_001.txt      # Request body for interaction 1
      ├── resp_001.txt     # Response body for interaction 1
      ├── req_002.b64      # Base64-encoded request body for interaction 2
      └── resp_002.txt     # Response body for interaction 2
  ```

## Usage

### Creating a Directory-Based Cassette

```rust
use http_client_vcr::{VcrClient, VcrMode, CassetteFormat};

// Using VcrClientBuilder
let vcr_client = VcrClient::builder("my_cassette_directory")
    .inner_client(Box::new(surf::client()))
    .mode(VcrMode::Record)
    .format(CassetteFormat::Directory)  // Specify directory format
    .build()
    .await?;
```

### Creating a Cassette Directly

```rust
use http_client_vcr::{Cassette, CassetteFormat};

let cassette = Cassette::new()
    .with_path(PathBuf::from("my_cassette_directory"))
    .with_format(CassetteFormat::Directory);
```

### Automatic Format Detection

The library automatically detects the format when loading existing cassettes:

```rust
// Automatically detects format based on path
let cassette = Cassette::load_from_file(path).await?;

// Works with both:
// - "cassette.yaml" (file format)
// - "cassette_directory/" (directory format)
```

## Directory Structure Details

### interactions.yaml
Contains interaction metadata without bodies:

```yaml
- request:
    method: POST
    url: "https://api.example.com/users"
    headers:
      content-type: ["application/json"]
    body_file: req_001.txt  # Reference to body file
    version: "HTTP/1.1"
  response:
    status: 201
    headers:
      content-type: ["application/json"]
    body_file: resp_001.txt  # Reference to body file
    version: "HTTP/1.1"
```

### Body Files
- **Text content**: Stored as `.txt` files
- **Base64 content**: Stored as `.b64` files
- **Naming**: `req_NNN.ext` for requests, `resp_NNN.ext` for responses
- **Empty bodies**: No file created, `body_file` field omitted

## Advantages of Directory Format

1. **Large Payloads**: Bodies aren't embedded in YAML, avoiding parsing issues
2. **Binary Content**: Base64 content stored in separate files
3. **Inspection**: Easy to view/edit individual response bodies
4. **Diff-Friendly**: Changes to bodies don't affect metadata file
5. **Selective Loading**: Could be extended to lazy-load bodies on demand

## Migration

Existing file-format cassettes continue to work without changes. To convert:

```rust
// Load existing file format
let cassette = Cassette::load_from_file("old_cassette.yaml").await?;

// Save as directory format
let dir_cassette = cassette
    .with_path(PathBuf::from("new_cassette_directory"))
    .with_format(CassetteFormat::Directory);
    
dir_cassette.save_to_file().await?;
```

## Backward Compatibility

- All existing APIs continue to work unchanged
- Default format remains `CassetteFormat::File`
- Format detection is automatic when loading
- Both formats can be used simultaneously in the same project