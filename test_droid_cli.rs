use splitrail_dashboard::Analyzer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create runtime
    let rt = tokio::runtime::Runtime::new()?;
    
    rt.block_on(async {
        // Test the Droid CLI analyzer on the actual data
        let analyzer = splitrail_dashboard::analyzers::droid_cli::DroidCliAnalyzer::new();
        
        // Check if available
        if !analyzer.is_available() {
            println!("❌ Droid CLI analyzer is not available (no data found)");
            return Ok(());
        }
        
        println!("✅ Droid CLI analyzer is available");
        
        // Get stats
        let stats = analyzer.get_stats().await?;
        
        println!("📊 Analyzer: {}", stats.analyzer_name);
        println!("📈 Total conversations: {}", stats.num_conversations);
        println!("💬 Total messages: {}", stats.messages.len());
        
        // Show message breakdown
        let mut user_messages = 0;
        let mut assistant_messages = 0;
        let mut total_cost = 0.0;
        let mut total_input_tokens = 0;
        let mut total_output_tokens = 0;
        
        for msg in &stats.messages {
            match msg.role {
                splitrail_dashboard::types::MessageRole::User => user_messages += 1,
                splitrail_dashboard::types::MessageRole::Assistant => assistant_messages += 1,
            }
            total_cost += msg.stats.cost;
            total_input_tokens += msg.stats.input_tokens;
            total_output_tokens += msg.stats.output_tokens;
        }
        
        println!("👤 User messages: {}", user_messages);
        println!("🤖 Assistant messages: {}", assistant_messages);
        println!("💰 Total cost: ${:.6}", total_cost);
        println!("📥 Total input tokens: {}", total_input_tokens);
        println!("📤 Total output tokens: {}", total_output_tokens);
        
        // Show daily breakdown
        if !stats.daily_stats.is_empty() {
            println!("\n📅 Daily breakdown:");
            for (date, daily) in &stats.daily_stats {
                println!("  {}: {} messages, ${:.6}", date, daily.user_messages + daily.ai_messages, daily.stats.cost);
            }
        }
        
        // Show first few messages with timestamps
        println!("\n📝 First 5 messages:");
        for (i, msg) in stats.messages.iter().take(5).enumerate() {
            println!("  {}. {} [{}] - {} tokens - ${:.6}", 
                i + 1,
                msg.date.format("%Y-%m-%d %H:%M:%S UTC"),
                match msg.role {
                    splitrail_dashboard::types::MessageRole::User => "User",
                    splitrail_dashboard::types::MessageRole::Assistant => "Assistant",
                },
                msg.stats.input_tokens + msg.stats.output_tokens,
                msg.stats.cost
            );
            if let Some(content) = &msg.content {
                let preview: String = content.chars().take(80).collect();
                println!("     Preview: {}{}", preview, if content.len() > 80 { "..." } else { "" });
            }
        }
        
        Ok(())
    })
}