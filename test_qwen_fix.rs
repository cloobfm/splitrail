use splitrail_dashboard::analyzers::qwen_code::QwenCodeAnalyzer;
use splitrail_dashboard::analyzer::Analyzer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let analyzer = QwenCodeAnalyzer::new();
    
    if analyzer.is_available() {
        println!("Qwen Code analyzer is available");
        
        let stats = analyzer.get_stats().await?;
        println!("Total messages: {}", stats.messages.len());
        
        // Find first message with tokens to check the fix
        for msg in &stats.messages {
            if msg.stats.input_tokens > 0 {
                println!("Found message with tokens:");
                println!("  Input tokens: {}", msg.stats.input_tokens);
                println!("  Reasoning tokens: {}", msg.stats.reasoning_tokens);
                println!("  Output tokens: {}", msg.stats.output_tokens);
                println!("  Cached tokens: {}", msg.stats.cached_tokens);
                println!("  Cost: ${:.6}", msg.stats.cost);
                println!("  Model: {:?}", msg.model);
                break;
            }
        }
    } else {
        println!("Qwen Code analyzer is not available");
    }
    
    Ok(())
}