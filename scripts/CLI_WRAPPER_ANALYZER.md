# CLI Wrapper Analyzer

A comprehensive analysis tool for CLI wrapper applications like **Kiro CLI** and **Amazon Q** that intercept shell commands.

## Overview

This tool analyzes the installation patterns and system integration of CLI wrapper tools that use identical architectures to intercept and enhance terminal commands.

### What Are CLI Wrappers?

CLI wrapper tools like Kiro CLI and Amazon Q install themselves deep into your system using a consistent pattern:

1. **Application Bundle** - Install in `/Applications/`
2. **Data Storage** - Create directory in `~/Library/Application Support/`
3. **Shell Hijacking** - Replace shell binaries in `~/.local/bin/` to intercept commands
4. **Auto-loading** - Inject initialization scripts into shell RC files

## Key Findings

### Identical Architecture

Both Amazon Q and Kiro CLI use **exactly the same** installation pattern:

```
~/Library/Application Support/<tool-name>/
├── data.sqlite3          # SQLite database (identical schema)
├── history               # Command history
├── settings.json         # Configuration
└── shell/               # Shell integration hooks
    ├── bash_login.pre.bash
    ├── bash_login.post.bash
    ├── bashrc.pre.bash
    ├── bashrc.post.bash
    ├── zshrc.pre.zsh
    ├── zshrc.post.zsh
    └── ... (12 files total)

/Applications/<Tool Name>.app/
└── Contents/
    └── MacOS/
        ├── <tool-cli>
        ├── <tool-cli-chat>
        └── <tool-cli-term>

~/.local/bin/
├── <tool-cli> -> /Applications/<Tool>.app/...
├── <tool-cli-term> -> /Applications/<Tool>.app/...
├── bash (<tool-cli-term>)    # 87MB wrapper binary
├── zsh (<tool-cli-term>)     # 87MB wrapper binary
├── fish (<tool-cli-term>)    # 87MB wrapper binary
└── nu (<tool-cli-term>)      # 87MB wrapper binary
```

### Database Schema

Both tools use **identical SQLite schemas** with these tables:
- `auth_kv` - Authentication key-value store
- `conversations` - Chat/conversation history
- `history` - Command history
- `migrations` - Database version tracking
- `state` - Application state

### Kiro CLI Has Taken Over Amazon Q

The analyzer discovered that **Kiro CLI has replaced Amazon Q**:

```bash
$ cat ~/.local/bin/q
#!/bin/sh
"/Users/bzl/.local/bin/kiro-cli" --show-legacy-warning "$@"
```

The `q` command (Amazon Q's CLI) now redirects to `kiro-cli` with a legacy warning.

## Usage

### Basic Analysis

```bash
./analyze_cli_wrappers.sh
```

Analyzes both Amazon Q and Kiro CLI installations, showing:
- Data directory contents and sizes
- Application bundle locations
- CLI binaries and symlinks
- Shell wrapper status
- Shell integration hooks

### Detailed Comparison

```bash
./analyze_cli_wrappers.sh --compare
```

Shows detailed comparison between the two tools:
- Database schema comparison
- File structure similarities
- Shell hook differences
- Takeover detection

### Removal Instructions

```bash
./analyze_cli_wrappers.sh --removal
```

Displays step-by-step instructions for completely removing these tools from your system.

## Example Output

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Analyzing: Amazon Q
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

▶ Data Directory
────────────────────────────────────────────────────────────
✓ Data directory exists: /Users/bzl/Library/Application Support/amazon-q
✓ Database: data.sqlite3 (5.8M)
✓ History: 2730 lines (73K)
✓ Shell integration: 12 hook files
ℹ Total size: 5.9M

▶ Shell Command Wrappers
────────────────────────────────────────────────────────────
⚠ Shell hijacked: bash (83.5MiB)
⚠ Shell hijacked: zsh (83.5MiB)
⚠ Shell hijacked: fish (83.5MiB)
⚠ Shell hijacked: nu (83.5MiB)
⚠ WARNING: Shell commands are being intercepted!
```

## What This Means

### Security Implications

These tools have **deep system integration**:
- ✓ All shell commands pass through their wrappers
- ✓ They can see every command you type
- ✓ They maintain a complete command history
- ✓ They auto-load on every new shell session

### Performance Impact

- **~350MB** of shell wrapper binaries in `~/.local/bin/`
- Shell startup time may increase (loading wrapper on each session)
- Every command goes through an additional layer

### Data Collection

Both tools maintain:
- Complete command history (unencrypted)
- SQLite database of conversations and state
- Shell integration metadata

## Removal

If you want to remove these tools completely:

```bash
# 1. Remove applications
rm -rf /Applications/"Amazon Q.app"
rm -rf /Applications/"Kiro CLI.app"

# 2. Remove data
rm -rf ~/Library/Application\ Support/amazon-q
rm -rf ~/Library/Application\ Support/kiro-cli

# 3. Remove CLI tools
rm ~/.local/bin/{q,qterm,kiro-cli,kiro-cli-chat,kiro-cli-term}

# 4. Remove shell wrappers
rm ~/.local/bin/"bash (qterm)"
rm ~/.local/bin/"zsh (qterm)"
rm ~/.local/bin/"fish (qterm)"
rm ~/.local/bin/"nu (qterm)"
rm ~/.local/bin/"bash (kiro-cli-term)"
rm ~/.local/bin/"zsh (kiro-cli-term)"
rm ~/.local/bin/"fish (kiro-cli-term)"
rm ~/.local/bin/"nu (kiro-cli-term)"

# 5. Clean shell RC files (remove sourcing lines)
# Edit: ~/.zshrc, ~/.bashrc, ~/.bash_profile, ~/.config/fish/config.fish

# 6. Restart shell
exec $SHELL -l
```

## Technical Details

### Shell Wrapper Mechanism

The shell wrappers work by:

1. Installing wrapper binaries that have the same name as your shell (`bash`, `zsh`, etc.)
2. Placing these in `~/.local/bin/` which is typically early in `$PATH`
3. The wrapper intercepts every command and can:
   - Log it to history
   - Analyze it for AI assistance
   - Modify it before execution
   - Inject additional functionality

### Shell Hook Loading

Each shell RC file gets these hooks injected:

```bash
# For zsh (~/.zshrc)
[ -x ~/.local/bin/kiro-cli ] && eval "$(~/.local/bin/kiro-cli init zsh post --rcfile zshrc)"

# For bash (~/.bashrc)
[ -x ~/.local/bin/kiro-cli ] && eval "$(~/.local/bin/kiro-cli init bash post --rcfile bashrc)"
```

These hooks:
- Check if the CLI exists
- Run the CLI's `init` command
- Evaluate the output to set up environment

## Adding New Wrapper Tools

To analyze additional CLI wrapper tools, edit the script and add to `KNOWN_WRAPPERS`:

```bash
KNOWN_WRAPPERS=(
    "amazon-q:Amazon Q:q:qterm"
    "kiro-cli:Kiro CLI:kiro-cli:kiro-cli-term"
    "new-tool:New Tool:newtool:newtool-term"
)
```

Format: `"data-dir-name:App Name:cli-binary:term-wrapper"`

## Requirements

- macOS (uses macOS-specific commands like `defaults`, `stat -f`)
- Bash 4.0+
- Optional: `jq` for JSON pretty-printing
- Optional: `sqlite3` for database schema analysis

## Author Notes

This analyzer was created after discovering that Kiro CLI had completely taken over Amazon Q's installation, using an identical architecture and even redirecting Amazon Q's `q` command to KIRO's binary.

The identical nature of these tools suggests they may share:
- A common codebase
- The same development team
- A standard CLI wrapper framework

## License

Part of the splitrail project. See main LICENSE file.
