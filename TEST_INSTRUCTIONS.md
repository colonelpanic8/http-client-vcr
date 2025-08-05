# Directory VCR Tests - Quick Start

## What we built
- Created `tests/directory_vcr_tests.rs` with 3 tests for directory-based cassette format
- Added reqwest adapter to work with http-client trait
- Environment variable control: `VCR_RECORD=1` to record, unset to replay

## Tests created
1. `test_multiple_requests_directory_format` - Makes 4 different HTTP requests
2. `test_repeated_requests_directory_format` - Same request 3 times  
3. `test_json_and_text_responses_directory_format` - Different content types

## To run
```bash
# Record new interactions (makes real HTTP calls)
VCR_RECORD=1 cargo test test_multiple_requests_directory_format

# Replay from cassette (no real HTTP calls)
cargo test test_multiple_requests_directory_format
```

## Status
- Tests compile and structure is ready
- Need to run with VCR_RECORD=1 to create initial cassettes
- Then run without to test replay functionality
- Directory cassettes will be created in `tests/fixtures/`

The implementation is simple - no complex auth like the lastfm reference, just basic HTTP client recording/replay with environment variable control.