# Amazon Q Schema

Schema and samples for Amazon Q SQLite database structure.

## Format Overview

Amazon Q stores data in a SQLite database located at:
- `~/Library/Application Support/amazon-q/data.sqlite3`

The database contains multiple tables for different types of data:
- `conversations` - JSON conversation data with project paths as keys
- `history` - Shell command history with working directory tracking
- `migrations` - Database migration tracking
- `auth_kv` - Authentication key-value pairs
- `state` - Application state storage

## Working Directory Tracking

Amazon Q tracks working directory information in:
- `history` table: `cwd` column for shell commands
- `conversations` table: `env_state.current_working_directory` in conversation JSON

## Database Schema

### conversations table
- **key**: Project path (e.g., `/Users/bzl/Virtius/project|`)
- **value**: JSON conversation data containing messages, tools, and metadata

### history table
- Stores shell command execution history
- Includes working directory (`cwd`), timestamps, and execution details

### Other tables
- `migrations`: Tracks database schema versions
- `auth_kv`: Stores authentication information
- `state`: Stores application state as key-value pairs

## File Structure

- `schema.json`: JSON Schema describing the SQLite database structure
- `samples/`: Sample conversation data extracted from real Amazon Q usage