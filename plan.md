# Bandcamp Collection to WebDAV Sync - Implementation Plan

## Overview
A Rust CLI tool that fetches your Bandcamp purchases/collection and syncs them with a WebDAV music library, organizing files by Artist/Album structure.

## Architecture

### Module Structure
```
src/
├── main.rs              # Entry point, CLI setup
├── cli/
│   ├── mod.rs          # CLI module exports
│   ├── commands.rs     # Command handlers
│   ├── auth.rs         # Authentication flow for user input
│   └── completions.rs  # Shell completion generation
├── bandcamp/
│   ├── mod.rs          # Bandcamp module exports
│   ├── client.rs       # HTTP client with cookie auth
│   ├── models.rs       # Data structures for API responses
│   ├── auth.rs         # Cookie/session management
│   ├── api.rs          # API endpoints (collection, downloads)
│   └── download.rs     # Album/track download logic
├── storage/
│   ├── mod.rs          # Storage module exports and trait definition
│   ├── webdav.rs       # WebDAV client implementation
│   ├── local.rs        # Local filesystem implementation
│   └── sync.rs         # Sync logic and file organization
├── reconciler/
│   ├── mod.rs          # Reconciliation logic
│   └── compare.rs      # Library comparison algorithms
└── utils/
    ├── mod.rs          # Utility exports
    ├── progress.rs     # Progress bars and spinners
    └── config.rs       # Configuration management

```

## Current Status (Phase 7/8 Complete)

### ✅ What's Working:
- **Authentication**: Browser-based login with fan_id extraction, 10-minute cookie expiry
- **Collection Fetching**: Successfully retrieves all 55 albums from Bandcamp API
- **Storage Backends**: Both local filesystem and WebDAV (basic) support
- **Dry-Run Mode**: Comprehensive preview with track counts, size estimates, cover art detection
- **Reconciliation**: Compares Bandcamp vs local/WebDAV, identifies missing albums
- **Smart Estimates**: Shows "?? tracks" when unknown, partial size estimates
- **Downloads**: Successfully downloads MP3 tracks from album pages
- **CLI**: All major commands work (auth, list, scan, diff, sync with actual downloads!)

### 🚧 What's Next:
- Polish and final testing
- Shell completions
- Full WebDAV listing support

## Key Design Decisions

### 1. Authentication Strategy
- **Bandcamp**: Multiple authentication options
  - Browser automation: Auto-login with provided credentials
  - Manual cookie: User provides cookie directly via flag/env var
  - Keyring storage: Optional storage in system keyring
  - All credentials can be provided via CLI flags or env vars
- **WebDAV**: Basic auth with username/password
  - Credentials via CLI flags or env vars
  - Optional keyring storage for convenience

### 2. Data Flow (Upsert-Only)
1. **Fetch Collection**: Get all purchases from Bandcamp API
2. **Scan WebDAV**: List existing music in WebDAV
3. **Reconcile**: Compare and identify missing albums
4. **Download**: Fetch missing albums from Bandcamp
5. **Upload**: Transfer to WebDAV with proper structure
   - **Important**: Only add/update, never remove from WebDAV
   - Albums removed from Bandcamp remain in WebDAV

### 3. Bandcamp API Integration
Based on research, we'll use these undocumented endpoints:
- `/api/fancollection/1/collection_items` - Get user's collection
- Track download URLs from purchase data
- Parse pagedata for fan_id and tokens

Key parameters:
- `fan_id`: User's fan ID from pagedata
- `older_than_token`: For pagination (format: `[timestamp]::a::`)
- `count`: Number of items to fetch

### 4. File Organization
```
Destination Root (WebDAV or Local Folder)/
├── Artist Name/
│   ├── Album Name/
│   │   ├── 01 - Track Title.aac
│   │   ├── 02 - Track Title.aac
│   │   └── cover.jpg
│   └── Another Album/
└── Another Artist/
```

**Destination Options**:
- WebDAV server URL (for network storage)
- Local folder path (for direct filesystem access)
- Both use identical Artist/Album structure

### 5. Audio Format Handling
- Primary: AAC (.m4a) for better quality/size ratio
- Fallback: MP3 if AAC unavailable
- Make format configurable via CLI flag
- Download highest quality available

### 6. CLI Interface
```bash
# Initial setup
bandcamp-sync auth bandcamp    # Prompts for cookie
bandcamp-sync auth webdav      # Prompts for WebDAV credentials

# Main commands
bandcamp-sync list              # List Bandcamp collection
bandcamp-sync scan              # Scan destination library
bandcamp-sync diff              # Show what's missing
bandcamp-sync sync              # Download and sync missing albums
bandcamp-sync sync --dry-run    # Preview what would be synced (REQUIRED for safety)

# Options
--format aac|mp3|flac          # Preferred audio format
--webdav-url URL               # WebDAV server URL (mutually exclusive with --local-path)
--local-path PATH              # Local folder path (mutually exclusive with --webdav-url)
--artist-filter PATTERN        # Filter by artist name
--album-filter PATTERN         # Filter by album name
--parallel N                   # Parallel downloads (default: 3)
--no-cover                     # Skip album art
--dry-run                      # Show what would be done without making changes
```

### 7. Dependencies
- `tokio`: Async runtime
- `clap`: CLI argument parsing
- `clap_complete`: Shell completions
- `reqwest`: HTTP client with cookie support
- `webdav`: WebDAV client library
- `serde`/`serde_json`: JSON parsing
- `scraper`: HTML parsing for pagedata
- `indicatif`: Progress bars
- `colored`: Terminal colors
- `keyring`: Secure credential storage
- `dialoguer`: Interactive prompts
- `tracing`: Logging
- `anyhow`/`thiserror`: Error handling

### 8. Error Handling
- Graceful handling of network failures
- Retry logic with exponential backoff
- Clear error messages for auth failures
- Resume capability for interrupted syncs

### 9. Progress Indication
- Spinner for API calls
- Progress bar for downloads
- Color-coded status messages:
  - Green: Success
  - Yellow: Warning/Skip
  - Red: Error
  - Blue: Info

### 10. Configuration & Storage
- **No config files**: All settings via CLI flags or environment variables
- **Credentials**: 
  - Can be provided via CLI flags or env vars
  - Optionally stored in system keyring for convenience
  - Keyring keys:
    - `bandcamp-sync:bandcamp:cookie` (with 10-minute expiry)
    - `bandcamp-sync:webdav:<url>:username`
    - `bandcamp-sync:webdav:<url>:password`
- **Environment variables**:
  - `BANDCAMP_USER`: Bandcamp username/email
  - `BANDCAMP_PASS`: Bandcamp password
  - `BANDCAMP_COOKIE`: Direct cookie value
  - `WEBDAV_URL`: WebDAV server URL
  - `WEBDAV_USER`: WebDAV username
  - `WEBDAV_PASS`: WebDAV password
  - `LOCAL_PATH`: Local folder destination path

### 11. Safety Features
- **Mandatory dry-run first**: Encourage users to always run with `--dry-run` before actual sync
- **Detailed preview**: Show exactly what will be downloaded and where it will go
- **No deletions**: Never remove files from destination (upsert-only)
- **Conflict detection**: Warn about files that would be overwritten
- **Size estimates**: Show total download size before proceeding

## Implementation Phases with Checkpoints

### Phase 1: Core Structure ✅ COMPLETED
- ✅ Set up project with dependencies
- ✅ Create module structure  
- ✅ Implement basic CLI with clap
- ✅ **CHECKPOINT**: `cargo build` - Ensure all modules compile
- ✅ **CHECKPOINT**: `cargo run -- --help` - Verify CLI works

### Phase 2: Authentication Module ✅ COMPLETED
- ✅ Implement browser-based Bandcamp auth with thirtyfour
- ✅ Implement WebDAV auth with credentials
- ✅ Keyring integration for secure storage
- ✅ **CHECKPOINT**: `cargo build` - Verify auth modules compile
- ✅ **CHECKPOINT**: Test auth commands work (without actual login)

### Phase 3: Bandcamp Module ✅ COMPLETED
- ✅ Create models for Bandcamp data structures
- ✅ Implement API client with cookie auth
- ✅ Parse collection/purchases endpoint
- ✅ **CHECKPOINT**: `cargo build` - Ensure Bandcamp module compiles
- ✅ **CHECKPOINT**: Test with real data (list command works!)

### Phase 4: Storage Module (WebDAV & Local) ✅ COMPLETED
- ✅ Create abstract storage trait for both WebDAV and local filesystem
- ✅ WebDAV client implementation using reqwest_dav (basic)
- ✅ Local filesystem implementation
- ✅ Directory listing and scanning for both backends
- ✅ File upload/write functionality with dry-run support
- ✅ **CHECKPOINT**: `cargo build` - Storage module compiles!
- ✅ **CHECKPOINT**: Test with local folder works
- ⏳ **TODO**: Full WebDAV listing implementation (currently returns empty)

### Phase 5: Core Functionality ✅ COMPLETED
- ✅ Implement list command (show Bandcamp collection - 55 albums!)
- ✅ Implement scan command (shows library contents)
- ✅ Implement diff command (compares collections)
- ✅ **CHECKPOINT**: `cargo run -- list` - Shows all 55 albums with proper Unicode!
- ✅ **CHECKPOINT**: `cargo run -- scan --local-path /tmp/test` - Works!
- ✅ **CHECKPOINT**: `cargo run -- diff --local-path /tmp/test` - Shows missing albums!

### Phase 6: Reconciliation & Sync ✅ MOSTLY COMPLETE
- ✅ Compare collections algorithm
- ✅ Implement comprehensive dry-run mode showing:
  - ✅ Albums and tracks to download (with "?? tracks" for unknowns)
  - ✅ Destination paths that would be created
  - ✅ Size estimates (marked as partial when incomplete)
  - ✅ Potential conflicts detection
  - ✅ Cover art detection from API
- ✅ **CHECKPOINT**: `cargo run -- diff --local-path /tmp/test` - Works perfectly!
- ✅ **CHECKPOINT**: `cargo run -- sync --dry-run --local-path /tmp/test` - Beautiful output!
- ⏳ **TODO**: Actual download from Bandcamp
- ⏳ **TODO**: Upload to destination with Artist/Album structure

### Phase 7: Download Implementation ✅ COMPLETED
- ✅ Fetch album pages to get download URLs
- ✅ Parse TralbumData from album pages (in data-tralbum attribute)
- ✅ Download MP3 tracks directly from TralbumData
- ✅ Organize tracks with proper numbering
- ✅ Handle album art download
- ✅ Progress bars for sync operations
- ✅ **CHECKPOINT**: Download single album test (Nicky Flowers worked!)
- ✅ **CHECKPOINT**: Files properly organized in Artist/Album structure

### Phase 8: Polish & Testing ✓ FINAL
- ✅ Progress indicators with indicatif (spinners work!)
- ✅ Colored output (beautiful terminal UI)
- ⏳ Shell completions
- ✅ Error handling for auth failures
- ⏳ **FINAL CHECKPOINT**: Full integration test
- ⏳ **FINAL CHECKPOINT**: `cargo clippy` - No warnings
- ⏳ **FINAL CHECKPOINT**: `cargo fmt` - Proper formatting

## Security Considerations
- Never log sensitive data (cookies, passwords)
- Use keyring for credential storage
- Validate SSL certificates
- Sanitize file names for filesystem safety

## Testing Strategy
- Unit tests for each module
- Integration tests with mock servers
- Manual testing with real Bandcamp/WebDAV
- Test various error conditions

## Future Enhancements
- Watch mode for automatic syncing
- Metadata enrichment (tags, lyrics)
- Duplicate detection
- Bandwidth limiting
- Multiple WebDAV targets
- Export collection as JSON/CSV