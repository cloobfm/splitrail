use splitrail_dashboard::analyzers::droid_cli::DroidCliAnalyzer;
use splitrail_dashboard::Analyzer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let analyzer = DroidCliAnalyzer::new();
    
    if !analyzer.is_available() {
        println!("❌ Droid CLI analyzer not available");
        return Ok(());
    }
    
    let stats = analyzer.get_stats().await?;
    
    println!("📊 First 10 messages (showing content):\n");
    
    for (i, msg) in stats.messages.iter().take(10).enumerate() {
        println!("{}. {} [{}] - {} tokens", 
            i + 1,
            msg.date.format("%H:%M:%S"),
            match msg.role {
                splitrail_dashboard::types::MessageRole::User => "User",
                splitrail_dashboard::types::MessageRole::Assistant => "Assistant",
            },
            msg.stats.input_tokens + msg.stats.output_tokens
        );
        
        if let Some(content) = &msg.content {
            if content == "Empty message" {
                println!("   📝 Content: [Empty message]");
            } else {
                // Show first 150 characters
                let preview = if content.len() > 150 {
                    format!("{}...", &content[..150])
                } else {
                    content.clone()
                };
                // Replace newlines with spaces for cleaner output
                let preview = preview.replace('\n', " ");
                println!("   📝 Content: {}", preview);
            }
        }
        println!();
    }
    
    // Count empty messages
    let empty_count = stats.messages.iter()
        .filter(|msg| {
            msg.content.as_ref()
                .map_or(false, |c| c == "Empty message")
        })
        .count();
    
    println!("📈 Summary:");
    println!("   Total messages: {}", stats.messages.len());
    println!("   Empty messages: {}", empty_count);
    println!("   Non-empty messages: {}", stats.messages.len() - empty_count);
    
    Ok(())
}