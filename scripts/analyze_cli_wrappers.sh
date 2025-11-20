#!/bin/bash
#
# CLI Wrapper Analyzer
# Analyzes Kiro CLI and Amazon Q (and similar CLI wrapper tools) installation patterns
# 
# These tools use an identical pattern:
# 1. Install app bundle in /Applications
# 2. Create data directory in ~/Library/Application Support
# 3. Install shell wrappers in ~/.local/bin that intercept shell commands
# 4. Inject shell init scripts to auto-load the wrapper
#

set -euo pipefail

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Configuration
KNOWN_WRAPPERS=(
    "amazon-q:Amazon Q:q:qterm"
    "kiro-cli:Kiro CLI:kiro-cli:kiro-cli-term"
)

print_header() {
    echo -e "\n${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BOLD}${CYAN}$1${NC}"
    echo -e "${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}\n"
}

print_section() {
    echo -e "\n${BOLD}${BLUE}▶ $1${NC}"
    echo -e "${BLUE}────────────────────────────────────────────────────────────${NC}"
}

print_item() {
    local status=$1
    shift
    case $status in
        found)
            echo -e "${GREEN}✓${NC} $*"
            ;;
        missing)
            echo -e "${YELLOW}○${NC} $*"
            ;;
        warning)
            echo -e "${YELLOW}⚠${NC} $*"
            ;;
        error)
            echo -e "${RED}✗${NC} $*"
            ;;
        info)
            echo -e "${CYAN}ℹ${NC} $*"
            ;;
    esac
}

format_size() {
    local size=$1
    if command -v numfmt &> /dev/null; then
        numfmt --to=iec-i --suffix=B "$size" 2>/dev/null || echo "${size}B"
    else
        echo "$size bytes"
    fi
}

analyze_wrapper() {
    local data_dir_name=$1
    local app_name=$2
    local cli_name=$3
    local term_name=$4
    
    print_header "Analyzing: $app_name"
    
    local found_something=0
    
    # Check data directory
    print_section "Data Directory"
    local data_dir="$HOME/Library/Application Support/$data_dir_name"
    if [ -d "$data_dir" ]; then
        print_item found "Data directory exists: $data_dir"
        found_something=1
        
        # Check for database
        if [ -f "$data_dir/data.sqlite3" ]; then
            local db_size=$(stat -f%z "$data_dir/data.sqlite3" 2>/dev/null || echo "0")
            print_item found "Database: data.sqlite3 ($(format_size $db_size))"
        fi
        
        # Check for history
        if [ -f "$data_dir/history" ]; then
            local hist_size=$(stat -f%z "$data_dir/history" 2>/dev/null || echo "0")
            local hist_lines=$(wc -l < "$data_dir/history" 2>/dev/null || echo "0")
            print_item found "History: $hist_lines lines ($(format_size $hist_size))"
        fi
        
        # Check for shell integration
        if [ -d "$data_dir/shell" ]; then
            local shell_files=$(find "$data_dir/shell" -type f | wc -l | tr -d ' ')
            print_item found "Shell integration: $shell_files hook files"
            
            # Show hook files
            echo -e "  ${CYAN}Shell hooks:${NC}"
            ls "$data_dir/shell" | sed 's/^/    /'
        fi
        
        # Check for settings
        if [ -f "$data_dir/settings.json" ]; then
            print_item found "Settings: settings.json"
            if command -v jq &> /dev/null && [ -r "$data_dir/settings.json" ]; then
                echo -e "  ${CYAN}Content:${NC}"
                jq . "$data_dir/settings.json" 2>/dev/null | sed 's/^/    /' || cat "$data_dir/settings.json" | sed 's/^/    /'
            fi
        fi
        
        # Total size
        if command -v du &> /dev/null; then
            local total_size=$(du -sh "$data_dir" 2>/dev/null | awk '{print $1}')
            print_item info "Total size: $total_size"
        fi
    else
        print_item missing "Data directory not found: $data_dir"
    fi
    
    # Check application bundle
    print_section "Application Bundle"
    local app_path="/Applications/$app_name.app"
    if [ -d "$app_path" ]; then
        print_item found "Application: $app_path"
        found_something=1
        
        # Check MacOS binaries
        local macos_dir="$app_path/Contents/MacOS"
        if [ -d "$macos_dir" ]; then
            echo -e "  ${CYAN}Binaries:${NC}"
            ls -lh "$macos_dir" | tail -n +2 | awk '{printf "    %s (%s)\n", $9, $5}'
        fi
        
        # Check version if Info.plist exists
        local plist="$app_path/Contents/Info.plist"
        if [ -f "$plist" ] && command -v defaults &> /dev/null; then
            local version=$(defaults read "$plist" CFBundleShortVersionString 2>/dev/null || echo "unknown")
            print_item info "Version: $version"
        fi
    else
        print_item missing "Application not found: $app_path"
    fi
    
    # Check CLI binaries and wrappers
    print_section "CLI Binaries & Shell Wrappers"
    local local_bin="$HOME/.local/bin"
    
    # Main CLI binary
    if [ -L "$local_bin/$cli_name" ] || [ -f "$local_bin/$cli_name" ]; then
        if [ -L "$local_bin/$cli_name" ]; then
            local target=$(readlink "$local_bin/$cli_name")
            print_item found "CLI (symlink): $cli_name -> $target"
        else
            local size=$(stat -f%z "$local_bin/$cli_name" 2>/dev/null || echo "0")
            print_item found "CLI (binary): $cli_name ($(format_size $size))"
        fi
        found_something=1
    else
        print_item missing "CLI binary not found: $local_bin/$cli_name"
    fi
    
    # Terminal wrapper
    if [ -L "$local_bin/$term_name" ] || [ -f "$local_bin/$term_name" ]; then
        if [ -L "$local_bin/$term_name" ]; then
            local target=$(readlink "$local_bin/$term_name")
            print_item found "Terminal wrapper (symlink): $term_name -> $target"
        else
            local size=$(stat -f%z "$local_bin/$term_name" 2>/dev/null || echo "0")
            print_item found "Terminal wrapper (binary): $term_name ($(format_size $size))"
        fi
    fi
    
    # Check for shell wrappers (bash, zsh, fish, nu)
    print_section "Shell Command Wrappers"
    local wrapper_found=0
    for shell in bash zsh fish nu; do
        local wrapper_file="$local_bin/$shell ($term_name)"
        if [ -f "$wrapper_file" ]; then
            local size=$(stat -f%z "$wrapper_file" 2>/dev/null || echo "0")
            print_item warning "Shell hijacked: $shell ($(format_size $size))"
            wrapper_found=1
        fi
    done
    
    if [ $wrapper_found -eq 0 ]; then
        print_item info "No shell command wrappers found"
    else
        print_item warning "${RED}${BOLD}WARNING: Shell commands are being intercepted!${NC}"
    fi
    
    # Check shell initialization
    print_section "Shell Integration Status"
    local rcfiles=(
        "$HOME/.zshrc:zsh"
        "$HOME/.bashrc:bash"
        "$HOME/.bash_profile:bash"
        "$HOME/.config/fish/config.fish:fish"
    )
    
    for entry in "${rcfiles[@]}"; do
        IFS=':' read -r rcfile shell_type <<< "$entry"
        if [ -f "$rcfile" ]; then
            if grep -q "$data_dir_name" "$rcfile" 2>/dev/null || grep -q "$cli_name" "$rcfile" 2>/dev/null; then
                print_item found "$shell_type: Hooks detected in $(basename "$rcfile")"
                # Show the actual hook line
                local hook_line=$(grep -E "$data_dir_name|$cli_name" "$rcfile" | head -1)
                echo -e "  ${CYAN}→${NC} $hook_line"
            fi
        fi
    done
    
    # Summary
    print_section "Summary"
    if [ $found_something -eq 1 ]; then
        print_item found "${BOLD}$app_name is installed and active${NC}"
        
        # Calculate total footprint
        local total_size=0
        if [ -d "$data_dir" ]; then
            total_size=$((total_size + $(du -sk "$data_dir" 2>/dev/null | awk '{print $1}' || echo 0)))
        fi
        if [ -d "$app_path" ]; then
            total_size=$((total_size + $(du -sk "$app_path" 2>/dev/null | awk '{print $1}' || echo 0)))
        fi
        
        if [ $total_size -gt 0 ]; then
            echo -e "  ${CYAN}Total footprint:${NC} $((total_size / 1024)) MB"
        fi
    else
        print_item missing "$app_name is not installed"
    fi
}

compare_wrappers() {
    print_header "Wrapper Comparison Analysis"
    
    print_section "Structural Similarity"
    print_item info "Both Amazon Q and Kiro CLI use identical installation patterns:"
    echo "  • Data in ~/Library/Application Support/<name>/"
    echo "  • SQLite database (data.sqlite3) for state"
    echo "  • Command history file"
    echo "  • Shell integration hooks in shell/ subdirectory"
    echo "  • Application bundle in /Applications/"
    echo "  • CLI symlinks in ~/.local/bin/"
    echo "  • Shell wrapper binaries that intercept commands"
    
    print_section "Key Differences"
    
    # Compare data directories if both exist
    local aq_dir="$HOME/Library/Application Support/amazon-q"
    local kiro_dir="$HOME/Library/Application Support/kiro-cli"
    
    if [ -d "$aq_dir" ] && [ -d "$kiro_dir" ]; then
        print_item info "Data Directory Contents:"
        
        # Compare database sizes
        if [ -f "$aq_dir/data.sqlite3" ] && [ -f "$kiro_dir/data.sqlite3" ]; then
            local aq_db_size=$(stat -f%z "$aq_dir/data.sqlite3")
            local kiro_db_size=$(stat -f%z "$kiro_dir/data.sqlite3")
            echo -e "  Amazon Q database: $(format_size $aq_db_size)"
            echo -e "  Kiro CLI database: $(format_size $kiro_db_size)"
        fi
        
        # Compare history
        if [ -f "$aq_dir/history" ] && [ -f "$kiro_dir/history" ]; then
            local aq_hist=$(wc -l < "$aq_dir/history" | tr -d ' ')
            local kiro_hist=$(wc -l < "$kiro_dir/history" | tr -d ' ')
            echo -e "  Amazon Q history: $aq_hist commands"
            echo -e "  Kiro CLI history: $kiro_hist commands"
        fi
        
        # Check if they're using the same database schema
        print_item info "Checking database schema similarity..."
        if command -v sqlite3 &> /dev/null; then
            local aq_tables=$(sqlite3 "$aq_dir/data.sqlite3" ".tables" 2>/dev/null | tr ' ' '\n' | sort)
            local kiro_tables=$(sqlite3 "$kiro_dir/data.sqlite3" ".tables" 2>/dev/null | tr ' ' '\n' | sort)
            
            if [ "$aq_tables" = "$kiro_tables" ]; then
                print_item found "Database schemas are IDENTICAL"
                echo -e "  ${CYAN}Tables:${NC}"
                echo "$aq_tables" | sed 's/^/    /'
            else
                print_item warning "Database schemas differ"
            fi
        fi
        
        # Compare shell hooks
        print_item info "Comparing shell integration hooks..."
        if [ -d "$aq_dir/shell" ] && [ -d "$kiro_dir/shell" ]; then
            local aq_hooks=$(ls "$aq_dir/shell" | sort)
            local kiro_hooks=$(ls "$kiro_dir/shell" | sort)
            
            if [ "$aq_hooks" = "$kiro_hooks" ]; then
                print_item found "Shell hook files are IDENTICAL in structure"
                
                # Check content differences
                local diff_count=0
                for hook in $(ls "$aq_dir/shell"); do
                    if ! diff -q "$aq_dir/shell/$hook" "$kiro_dir/shell/$hook" &>/dev/null; then
                        ((diff_count++))
                    fi
                done
                
                echo -e "  ${CYAN}Hook files differ only in binary name references${NC}"
            fi
        fi
    fi
    
    print_section "Relationship Analysis"
    
    # Check if one is redirecting to the other
    if [ -f "$HOME/.local/bin/q" ]; then
        print_item info "Checking Amazon Q CLI wrapper..."
        local q_content=$(cat "$HOME/.local/bin/q")
        if echo "$q_content" | grep -q "kiro-cli"; then
            print_item warning "${YELLOW}${BOLD}Amazon Q CLI ('q') redirects to Kiro CLI!${NC}"
            echo -e "  ${CYAN}Content:${NC}"
            cat "$HOME/.local/bin/q" | sed 's/^/    /'
            echo ""
            print_item warning "${RED}${BOLD}Kiro CLI has taken over Amazon Q${NC}"
        fi
    fi
}

show_removal_instructions() {
    print_header "Removal Instructions"
    
    echo -e "${YELLOW}To completely remove a CLI wrapper tool:${NC}\n"
    
    echo -e "${BOLD}1. Remove the application bundle:${NC}"
    echo "   rm -rf /Applications/\"Amazon Q.app\""
    echo "   rm -rf /Applications/\"Kiro CLI.app\""
    
    echo -e "\n${BOLD}2. Remove data directories:${NC}"
    echo "   rm -rf ~/Library/Application\\ Support/amazon-q"
    echo "   rm -rf ~/Library/Application\\ Support/kiro-cli"
    
    echo -e "\n${BOLD}3. Remove CLI binaries and symlinks:${NC}"
    echo "   rm ~/.local/bin/q"
    echo "   rm ~/.local/bin/qterm"
    echo "   rm ~/.local/bin/kiro-cli"
    echo "   rm ~/.local/bin/kiro-cli-chat"
    echo "   rm ~/.local/bin/kiro-cli-term"
    
    echo -e "\n${BOLD}4. Remove shell wrappers:${NC}"
    echo "   rm ~/.local/bin/\"bash (qterm)\""
    echo "   rm ~/.local/bin/\"zsh (qterm)\""
    echo "   rm ~/.local/bin/\"fish (qterm)\""
    echo "   rm ~/.local/bin/\"nu (qterm)\""
    echo "   rm ~/.local/bin/\"bash (kiro-cli-term)\""
    echo "   rm ~/.local/bin/\"zsh (kiro-cli-term)\""
    echo "   rm ~/.local/bin/\"fish (kiro-cli-term)\""
    echo "   rm ~/.local/bin/\"nu (kiro-cli-term)\""
    
    echo -e "\n${BOLD}5. Clean up shell configuration files:${NC}"
    echo "   # Remove sourcing lines from:"
    echo "   #   ~/.zshrc"
    echo "   #   ~/.bashrc"
    echo "   #   ~/.bash_profile"
    echo "   #   ~/.config/fish/config.fish"
    
    echo -e "\n${BOLD}6. Restart your shell${NC}"
    echo "   exec \$SHELL -l"
}

# Main execution
main() {
    local show_comparison=0
    local show_removal=0
    
    # Parse arguments
    for arg in "$@"; do
        case $arg in
            --compare)
                show_comparison=1
                ;;
            --removal)
                show_removal=1
                ;;
            --help|-h)
                echo "CLI Wrapper Analyzer"
                echo ""
                echo "Usage: $0 [options]"
                echo ""
                echo "Options:"
                echo "  --compare   Show detailed comparison between wrappers"
                echo "  --removal   Show removal instructions"
                echo "  --help      Show this help message"
                exit 0
                ;;
        esac
    done
    
    print_header "CLI Wrapper Analysis Tool"
    echo -e "${CYAN}Analyzing installation patterns for CLI wrapper tools...${NC}"
    
    # Analyze each known wrapper
    for wrapper in "${KNOWN_WRAPPERS[@]}"; do
        IFS=':' read -r data_dir app_name cli_name term_name <<< "$wrapper"
        analyze_wrapper "$data_dir" "$app_name" "$cli_name" "$term_name"
    done
    
    # Show comparison if requested or if both are installed
    if [ $show_comparison -eq 1 ] || ([ -d "$HOME/Library/Application Support/amazon-q" ] && [ -d "$HOME/Library/Application Support/kiro-cli" ]); then
        compare_wrappers
    fi
    
    # Show removal instructions if requested
    if [ $show_removal -eq 1 ]; then
        show_removal_instructions
    fi
    
    print_header "Analysis Complete"
    echo -e "${CYAN}Run with --compare to see detailed comparison${NC}"
    echo -e "${CYAN}Run with --removal to see removal instructions${NC}"
}

# Run main function
main "$@"
