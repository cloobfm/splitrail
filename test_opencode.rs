use std::env;

fn main() {
    // Set home directory for testing
    if let Some(home) = env::var_os("HOME") {
        let home_str = home.to_string_lossy();
        println!("Home directory: {}", home_str);
        
        // Check if OpenCode directories exist
        let config_path = format!("{}/.config/opencode", home_str);
        let local_share_path = format!("{}/.local/share/opencode", home_str);
        
        println!("Config path exists: {}", std::path::Path::new(&config_path).exists());
        println!("Local share path exists: {}", std::path::Path::new(&local_share_path).exists());
        
        // Check for data files
        let part_pattern = format!("{}/.local/share/opencode/storage/part/msg_*/prt_*.json", home_str);
        println!("Part pattern: {}", part_pattern);
        
        let session_pattern = format!("{}/.local/share/opencode/storage/session/*/ses_*.json", home_str);
        println!("Session pattern: {}", session_pattern);
        
        // Use glob to find files
        if let Ok(entries) = glob::glob(&part_pattern) {
            let count = entries.count();
            println!("Found {} part files", count);
        }
        
        if let Ok(entries) = glob::glob(&session_pattern) {
            let count = entries.count();
            println!("Found {} session files", count);
        }
    }
}