use splitrail_dashboard::analyzers::OpenCodeAnalyzer;
use splitrail_dashboard::analyzer::Analyzer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let analyzer = OpenCodeAnalyzer::new();
    
    println!("OpenCode Analyzer Test");
    println!("====================");
    
    // Check if analyzer is available
    let is_available = analyzer.is_available();
    println!("Analyzer available: {}", is_available);
    
    if is_available {
        // Get data patterns
        let patterns = analyzer.get_data_glob_patterns();
        println!("Data patterns:");
        for (i, pattern) in patterns.iter().enumerate() {
            println!("  {}: {}", i + 1, pattern);
        }
        
        // Discover data sources
        match analyzer.discover_data_sources() {
            Ok(sources) => {
                println!("Found {} data sources:", sources.len());
                for (i, source) in sources.iter().enumerate() {
                    println!("  {}: {}", i + 1, source.path.display());
                }
                
                // Get stats
                match analyzer.get_stats().await {
                    Ok(stats) => {
                        println!("Stats loaded successfully!");
                        println!("Total messages: {}", stats.messages.len());
                        println!("Total conversations: {}", stats.num_conversations);
                        println!("Daily stats entries: {}", stats.daily_stats.len());
                        
                        // Show some sample messages
                        if !stats.messages.is_empty() {
                            println!("Sample messages:");
                            for (i, msg) in stats.messages.iter().take(3).enumerate() {
                                println!("  {}: Role: {:?}, Date: {}, Model: {:?}", 
                                    i + 1, msg.role, msg.date, msg.model);
                            }
                        }
                    }
                    Err(e) => {
                        println!("Error getting stats: {}", e);
                    }
                }
            }
            Err(e) => {
                println!("Error discovering data sources: {}", e);
            }
        }
    }
    
    Ok(())
}