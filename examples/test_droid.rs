use splitrail_dashboard::analyzers::droid_cli::DroidCliAnalyzer;
use splitrail_dashboard::Analyzer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let analyzer = DroidCliAnalyzer::new();
    
    if !analyzer.is_available() {
        println!("❌ Droid CLI analyzer not available");
        return Ok(());
    }
    
    println!("✅ Droid CLI analyzer available");
    
    let stats = analyzer.get_stats().await?;
    
    println!("📊 Total messages: {}", stats.messages.len());
    println!("📅 Daily stats entries: {}", stats.daily_stats.len());
    
    // Show message count by role
    let mut user_count = 0;
    let mut assistant_count = 0;
    
    for msg in &stats.messages {
        match msg.role {
            splitrail_dashboard::types::MessageRole::User => user_count += 1,
            splitrail_dashboard::types::MessageRole::Assistant => assistant_count += 1,
        }
    }
    
    println!("👤 User messages: {}", user_count);
    println!("🤖 Assistant messages: {}", assistant_count);
    
    // Show date range
    if let Some((first_date, _)) = stats.daily_stats.iter().next() {
        if let Some((last_date, _)) = stats.daily_stats.iter().last() {
            println!("📅 Date range: {} to {}", first_date, last_date);
        }
    }
    
    Ok(())
}