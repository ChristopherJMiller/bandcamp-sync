```
 ______                  _
(____  \                | |
 ____)  )_____ ____   __| | ____ _____ ____  ____
|  __  ((____ |  _ \ / _  |/ ___|____ |    \|  _ \
| |__)  ) ___ | | | ( (_| ( (___/ ___ | | | | |_| |
|______/\_____|_| |_|\____|\____)_____|_|_|_|  __/
                                            |_|
  ______
 / _____)
( (____  _   _ ____   ____
 \____ \| | | |  _ \ / ___)
 _____) ) |_| | | | ( (___
(______/ \__  |_| |_|\____)
        (____/
```

A CLI tool to sync your Bandcamp music collection to WebDAV storage or local folders.

## Features

- Sync to WebDAV servers (e.g., Nextcloud, ownCloud) or local folders
- Smart incremental sync (only downloads missing albums)
- Flexible filtering by artist or album
- Dry-run mode to preview changes

## Installation

### Using Nix Flake

```bash
nix develop
cargo build --release
```

### Requirements

- WebDriver for browser automation (one of):
  - Firefox: `geckodriver` (default)
  - Chrome: `chromedriver`
  - Safari: `safaridriver` (macOS only)
- System keyring support

## Quick Start

### 1. Start WebDriver

#### Firefox (default):

```bash
geckodriver --port=4444
```

#### Chrome:

```bash
chromedriver --port=9515
```

#### Safari (macOS):

```bash
safaridriver --port=4444
```

### 2. Authenticate with Bandcamp

```bash
# Uses Firefox by default
bandcamp-sync auth bandcamp

# Or explicitly specify a browser
bandcamp-sync auth bandcamp --driver chrome
bandcamp-sync auth bandcamp --driver safari
```

### 3. Sync Your Collection

#### To a local folder:

```bash
bandcamp-sync sync --local-path ~/Music
```

#### To WebDAV:

```bash
bandcamp-sync sync --webdav-url https://cloud.example.com/dav
```

## Common Workflows

### List Your Collection

```bash
# List all albums
bandcamp-sync list

# Filter by artist
bandcamp-sync list --artist-filter "JER"

# Exclude specific artists
bandcamp-sync list --exclude-artist "Various Artists"

# Output as JSON or CSV
bandcamp-sync list --format json
bandcamp-sync list --format csv
```

### Check What's Missing

```bash
# Compare Bandcamp collection with your storage
bandcamp-sync diff --local-path ~/Music

# Show only missing albums
bandcamp-sync diff --local-path ~/Music --missing-only
```

### Sync with Options

```bash
# Dry run to preview what would be downloaded
bandcamp-sync sync --local-path ~/Music --dry-run

# Download specific format
bandcamp-sync sync --local-path ~/Music --format flac

# Filter what to sync
bandcamp-sync sync --local-path ~/Music --artist-filter "Radiohead"

# Exclude artists
bandcamp-sync sync --local-path ~/Music --exclude-artist "Various"

# Disable parallel downloads
bandcamp-sync sync --local-path ~/Music --no-parallel

# Custom parallel workers
bandcamp-sync sync --local-path ~/Music --parallel 1337

# Skip cover art
bandcamp-sync sync --local-path ~/Music --no-cover
```

### Manual Import

For albums that fail to download automatically (e.g., hidden from artist's public page):

```bash
# Import a manually downloaded Bandcamp zip
bandcamp-sync import-zip ~/Downloads/album.zip --webdav-url https://dav.example.com

# Or to local storage
bandcamp-sync import-zip ~/Downloads/album.zip --local-path ~/Music
```

## Command Reference

### Global Options

- `-v, --verbose` - Enable verbose logging

### `auth bandcamp`

Authenticate with Bandcamp and store credentials.

**Options:**

- `--headless` - Run browser in headless mode
- `--driver <DRIVER>` - Browser driver to use: `firefox` (default), `chrome`, `safari` (env: `BROWSER_DRIVER`)
- `--driver-port <PORT>` - WebDriver port (defaults: Firefox 4444, Chrome 9515, Safari 4444) (env: `WEBDRIVER_PORT`)
- `--username <USER>` - Bandcamp username (can use env: `BANDCAMP_USER`)
- `--password <PASS>` - Bandcamp password (can use env: `BANDCAMP_PASS`)
- `--cookie <COOKIE>` - Provide cookie directly (env: `BANDCAMP_COOKIE`)
- `--force` - Force re-authentication even if valid cookie exists

### `auth webdav`

Authenticate with WebDAV server.

**Options:**

- `--url <URL>` - WebDAV URL (env: `WEBDAV_URL`)
- `--username <USER>` - Username (env: `WEBDAV_USER`)
- `--password <PASS>` - Password (env: `WEBDAV_PASS`)

### `list`

List your Bandcamp collection.

**Options:**

- `--artist-filter <PATTERN>` - Include only artists matching pattern
- `--exclude-artist <PATTERN>` - Exclude artists matching pattern
- `--album-filter <PATTERN>` - Filter by album name
- `--format <FORMAT>` - Output format: `table` (default), `json`, `csv`

### `scan`

Scan your storage to see what's already downloaded.

**Options:**

- `--webdav-url <URL>` - WebDAV URL (env: `WEBDAV_URL`)
- `--local-path <PATH>` - Local folder path (env: `LOCAL_PATH`)
- `--detailed` - Show detailed information including tracks

### `diff`

Compare Bandcamp collection with your storage.

**Options:**

- `--webdav-url <URL>` - WebDAV URL (env: `WEBDAV_URL`)
- `--local-path <PATH>` - Local folder path (env: `LOCAL_PATH`)
- `--missing-only` - Show only missing albums
- `--artist-filter <PATTERN>` - Include only artists matching pattern
- `--exclude-artist <PATTERN>` - Exclude artists matching pattern

### `sync`

Sync missing albums from Bandcamp to your storage.

**Options:**

- `--webdav-url <URL>` - WebDAV URL (env: `WEBDAV_URL`)
- `--local-path <PATH>` - Local folder path (env: `LOCAL_PATH`)
- `--dry-run` - Preview what would be synced without downloading
- `--format <FORMAT>` - Audio format: `aac` (default), `mp3`, `flac`, `wav`
- `--parallel <N>` - Number of parallel downloads
- `--no-parallel` - Disable parallel downloads
- `--no-cover` - Skip downloading cover art
- `--artist-filter <PATTERN>` - Include only artists matching pattern
- `--exclude-artist <PATTERN>` - Exclude artists matching pattern
- `--album-filter <PATTERN>` - Filter by album name

### `status`

Check authentication status and cookie expiry.

## Storage Structure

Albums are organized in the following structure:

```
Storage Root/
├── Artist Name/
│   ├── Album Name/
│   │   ├── 01 - Track Name.m4a
│   │   ├── 02 - Track Name.m4a
│   │   └── cover.jpg
│   └── Another Album/
│       └── ...
└── Another Artist/
    └── ...
```

## Environment Variables

You can set these environment variables to avoid passing flags:

- `BANDCAMP_USER` - Bandcamp username
- `BANDCAMP_PASS` - Bandcamp password
- `BANDCAMP_COOKIE` - Bandcamp session cookie
- `BROWSER_DRIVER` - Browser driver to use (firefox, chrome, safari)
- `WEBDRIVER_PORT` - WebDriver port number
- `WEBDAV_URL` - WebDAV server URL
- `WEBDAV_USER` - WebDAV username
- `WEBDAV_PASS` - WebDAV password
- `LOCAL_PATH` - Local storage path

## Troubleshooting

### WebDriver Issues

If you get "Failed to connect to WebDriver":

#### Firefox (geckodriver) - Default

1. Install: `brew install geckodriver` (macOS) or download from [GitHub](https://github.com/mozilla/geckodriver/releases)
2. Start it: `geckodriver --port=4444`
3. Check it's running: `curl http://localhost:4444/status`

#### Chrome (chromedriver)

1. Install: `brew install chromedriver` (macOS) or download from [ChromeDriver website](https://chromedriver.chromium.org/)
2. Start it: `chromedriver --port=9515`
3. Check it's running: `curl http://localhost:9515/status`

#### Safari (safaridriver) - macOS only

1. Enable: `safaridriver --enable` (one-time setup)
2. Start it: `safaridriver --port=4444`
3. Note: Safari may require allowing remote automation in Safari's Develop menu
